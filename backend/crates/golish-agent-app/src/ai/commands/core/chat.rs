//! AI chat commands: send prompt (with attachments), clear conv, signal ready,
//! cancel, vision capabilities.

use crate::error::GolishError;
use std::sync::Arc;

use tauri::State;

use golish_agent_bridge::bridge_executor::UserIntent;

use super::super::super::agent_bridge::AgentBridge;
use crate::state::AgentState;

/// Send a prompt to the AI agent for a specific session.
///
/// This is the session-specific version of send_ai_prompt that routes to
/// the correct agent bridge based on session_id.
///
/// Execution mode dispatch:
/// - **Chat**: normal agentic loop (conversational with tools)
/// - **Task**: PentAGI-style automated orchestration (Generator → Subtasks → Refiner → Reporter)
///
/// IMPORTANT: Uses get_session_bridge() to clone the Arc and release the map
/// lock immediately. This allows other sessions to initialize/shutdown while
/// this session is executing, enabling true concurrent multi-tab agent execution.
#[tauri::command]
pub async fn send_ai_prompt_session(
    state: State<'_, AgentState>,
    session_id: String,
    prompt: String,
) -> Result<String, GolishError> {
    tracing::info!(
        message = "[send_ai_prompt_session] Received prompt",
        session_id = %session_id,
        prompt_len = prompt.len(),
    );

    // Get Arc clone and release map lock immediately
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| {
            tracing::error!(
                message = "[send_ai_prompt_session] Session not initialized",
                session_id = %session_id,
            );
            super::super::ai_session_not_initialized_error(&session_id)
        })?;

    let mode = bridge.get_execution_mode().await;

    tracing::info!(
        message = "[send_ai_prompt_session] Got bridge, executing prompt",
        session_id = %session_id,
        execution_mode = %mode,
    );

    match mode {
        golish_agent_kit::execution_mode::ExecutionMode::Chat => {
            bridge.execute(&prompt).await.map_err(|e| {
                tracing::error!(
                    message = "[send_ai_prompt_session] Chat execution error",
                    session_id = %session_id,
                    error = %e,
                );
                GolishError::Internal(e.to_string())
            })
        }
        golish_agent_kit::execution_mode::ExecutionMode::Task => {
            // Greeting/empty fast-path: skip the lead turn entirely (cheap) and let
            // the main agent reply directly — no point thinking about "hi".
            let user_message = extract_user_message_from_wrapped_prompt(&prompt);
            if deterministic_intent(user_message) == Some(UserIntent::Conversation) {
                tracing::info!(
                    message = "[send_ai_prompt_session] Greeting fast-path — main agent replies (no lead turn)",
                    session_id = %session_id,
                );
                return bridge
                    .execute(&prompt)
                    .await
                    .map_err(|e| GolishError::Internal(e.to_string()));
            }

            // D1=B (设计 2026-06-04): task mode goes straight into the operation
            // harness — there is NO separate "lead decision turn". The harness
            // cursor starts at `scoping` and the per-stage agent loop drives the
            // rest. This removes the path where a weak lead turn bypassed
            // `start_operation` and delegated straight to a `sub_agent_pentester`
            // (skipping scoping). Casual chat is handled by the greeting fast-path
            // above and by chat mode.
            execute_task_mode(bridge, &session_id, &prompt, &state)
                .await
                .map_err(|e| {
                    let error = format!("{:#}", e);
                    tracing::error!(
                        message = "[send_ai_prompt_session] Task execution error",
                        session_id = %session_id,
                        error = %error,
                    );
                    GolishError::Internal(error)
                })
        }
    }
}

/// Run Task mode orchestration (PentAGI-style).
///
/// Emits a short initial Started→TextDelta→Completed cycle so the frontend
/// immediately shows a response while the Generator LLM call runs.
/// Each subtask then manages its own Started/Completed lifecycle via
/// `execute_isolated` → `execute_with_context`.
async fn execute_task_mode(
    bridge: Arc<AgentBridge>,
    _session_id: &str,
    prompt: &str,
    state: &AgentState,
) -> anyhow::Result<String> {
    use anyhow::Context;
    use golish_agent_bridge::bridge_executor::BridgeAgentExecutor;
    use golish_agent_kit::task_orchestrator::TaskOrchestrator;
    use golish_core::events::AiEvent;
    use golish_db::{models::NewSession, repo::sessions};

    let task_input = extract_user_message_from_wrapped_prompt(prompt);

    // Anchor the chat-panel session string to one stable `sessions` row (Task
    // 断线恢复 · L1). Upserting by `chat_session_key` (instead of creating a row
    // per message) is what lets us find + resume this chat's prior operation
    // below; it also satisfies the `tasks.session_id` FK. Same chat session →
    // same DB session id on every message.
    let session_row = sessions::upsert_by_chat_key(
        &state.db_pool,
        _session_id,
        NewSession {
            title: Some(truncate_for_title(task_input, 80)),
            workspace_path: None,
            workspace_label: None,
            model: Some(bridge.model_name().to_string()),
            provider: Some(bridge.provider_name().to_string()),
            project_path: None,
        },
    )
    .await
    .context("Failed to upsert session row for task mode (FK precondition for tasks)")?;
    let uuid_session_id = session_row.id;
    tracing::info!(
        target: "harness::task_mode",
        session_db_id = %uuid_session_id,
        chat_session_id = %_session_id,
        "task mode session row created"
    );

    let event_tx = bridge.get_or_create_event_tx();

    // Echo the user's task input into the event stream (chat-mode parity).
    // Progress feedback is surfaced by the orchestrator's TaskProgress events.
    bridge.emit_event(AiEvent::UserMessage {
        content: task_input.to_string(),
    });

    let start_time = std::time::Instant::now();
    let db_repo: std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider> =
        std::sync::Arc::new(crate::ai::db_bridge::GolishDbRepoProvider::new(
            state.db_pool.clone(),
        ));
    let mut orchestrator = TaskOrchestrator::new(db_repo, uuid_session_id, event_tx);
    orchestrator.set_profile_override(bridge.get_harness_profile().await);
    let executor = BridgeAgentExecutor::new(bridge.clone());

    // Resume-aware entry (Task 断线恢复 · L2): if this chat session has a
    // non-terminal operation with a persisted harness checkpoint, resume it from
    // where it left off instead of starting a new run at scoping. The decision is
    // purely state-driven — the user's text ("继续" / anything / empty) is NOT
    // parsed as a keyword. `None` (nothing resumable) → fresh run.
    let resumable =
        golish_db::repo::tasks::latest_resumable_by_session(&state.db_pool, uuid_session_id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    target: "harness::task_mode",
                    error = %e,
                    "resume lookup failed; starting a fresh run"
                );
                None
            });

    let result = match resumable {
        Some(task) => {
            tracing::info!(
                target: "harness::task_mode",
                task_id = %task.id,
                "task mode: resuming prior operation for this chat session"
            );
            orchestrator.resume(task.id, task_input, &executor).await
        }
        None => orchestrator.run(task_input, &executor).await,
    };

    let duration_ms = start_time.elapsed().as_millis() as u64;

    match &result {
        Ok(response) => {
            tracing::info!(
                "[TaskMode] Completed in {:.1}s, report length: {} chars",
                duration_ms as f64 / 1000.0,
                response.len(),
            );
            // Emit the final report as a separate completed message
            let report_turn = uuid::Uuid::new_v4().to_string();
            bridge.emit_event(AiEvent::Started {
                turn_id: report_turn,
            });
            bridge.emit_event(AiEvent::TextDelta {
                delta: response.clone(),
                accumulated: response.clone(),
            });
            bridge.emit_event(AiEvent::Completed {
                response: response.clone(),
                reasoning: None,
                input_tokens: None,
                output_tokens: None,
                duration_ms: Some(duration_ms),
            });
        }
        Err(e) => {
            let err_msg = format!("{:#}", e);
            // Safety net: the planner could not produce a plan because the input
            // was conversational (model replied in prose / `{"message": …}`).
            // Answer via the main agent instead of surfacing a "Generator failed"
            // error. The deterministic triage in the caller catches most of these
            // before the planner runs; this handles the ambiguous misjudged ones.
            if is_conversational_planner_failure(&err_msg) {
                tracing::info!(
                    target: "harness::task_mode",
                    error = %err_msg,
                    "planner declined (conversational input) — falling back to main agent reply"
                );
                return bridge
                    .execute(prompt)
                    .await
                    .context("Chat fallback after planner declined");
            }
            bridge.emit_event(AiEvent::Error {
                message: err_msg,
                error_type: "task_orchestrator".to_string(),
            });
        }
    }

    result
}

/// Cheap, deterministic triage for the obvious cases so we don't pay an LLM
/// round-trip (and don't misfire when that call times out): empty input and pure
/// greetings/small-talk go to the main agent; clear security tasks go to the
/// planner. Genuinely ambiguous input returns `None` so the caller defers to the
/// LLM classifier. Conservative on purpose — only classify when confident.
fn deterministic_intent(user_message: &str) -> Option<UserIntent> {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return Some(UserIntent::Conversation);
    }
    let lower = trimmed.to_lowercase();

    // Clear security-task signals win (even if the message also says "hi").
    const TASK_SIGNALS: &[&str] = &[
        "http://",
        "https://",
        "scan ",
        "扫描",
        "exploit",
        "渗透",
        "pentest",
        "penetration",
        "enumerate",
        "recon",
        "侦察",
        "audit",
        "审计",
        "漏洞",
        "vulnerab",
        "brute",
        "爆破",
        "fuzz",
        "提权",
        "nmap",
        "sqlmap",
        "nikto",
        "gobuster",
        "subdomain",
        "子域",
    ];
    if TASK_SIGNALS.iter().any(|s| lower.contains(s)) {
        return Some(UserIntent::Task);
    }

    // Short, pure greetings / thanks → let the main agent reply directly.
    const CHAT_PREFIXES: &[&str] = &[
        "你好",
        "您好",
        "哈喽",
        "嗨",
        "在吗",
        "谢谢",
        "多谢",
        "感谢",
        "hi",
        "hello",
        "hey",
        "yo",
        "thanks",
        "thank you",
        "good morning",
        "good afternoon",
        "good evening",
        "早上好",
        "晚上好",
        "下午好",
    ];
    let short = trimmed.chars().count() <= 24;
    if short && CHAT_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return Some(UserIntent::Conversation);
    }

    // Identity / capability / definition questions are conversational.
    const CHAT_CONTAINS: &[&str] = &[
        "你是谁",
        "你能做什么",
        "你会什么",
        "你叫什么",
        "what can you do",
        "who are you",
        "how are you",
        "什么是",
        "what is",
        "explain ",
    ];
    if CHAT_CONTAINS.iter().any(|s| lower.contains(s)) {
        return Some(UserIntent::Conversation);
    }

    None
}

/// Whether a Task-orchestrator failure is really "the planner declined because
/// the input was conversational" (model replied in prose / `{"message": …}`
/// instead of a plan) rather than a genuine error. Mirrors the frontend
/// `classifyErrorSeverity` signals so both layers agree on what is "soft".
fn is_conversational_planner_failure(err_msg: &str) -> bool {
    let lower = err_msg.to_lowercase();
    const SIGNALS: &[&str] = &[
        "declined to produce a plan",
        "returned a message instead of a plan",
        "refused the request or asked a question",
        "failed to parse task planner json",
    ];
    if SIGNALS.iter().any(|s| lower.contains(s)) {
        return true;
    }
    lower.contains("missing field") && lower.contains("subtasks")
}

fn extract_user_message_from_wrapped_prompt(prompt: &str) -> &str {
    prompt
        .find("[User Message]\n")
        .map(|idx| &prompt[idx + "[User Message]\n".len()..])
        .unwrap_or(prompt)
        .trim()
}

/// Truncate a string to at most `max_bytes` bytes without splitting a
/// multi-byte UTF-8 character. Used to derive a short session title from
/// the user prompt; never panics on Chinese/emoji input.
fn truncate_for_title(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod chat_title_tests {
    use super::{
        deterministic_intent, extract_user_message_from_wrapped_prompt,
        is_conversational_planner_failure, truncate_for_title,
    };
    use golish_agent_bridge::bridge_executor::UserIntent;

    #[test]
    fn ascii_under_limit_returned_as_is() {
        assert_eq!(truncate_for_title("hello world", 80), "hello world");
    }

    #[test]
    fn ascii_over_limit_truncated_to_bytes() {
        let s = "a".repeat(200);
        let out = truncate_for_title(&s, 80);
        assert_eq!(out.len(), 80);
    }

    #[test]
    fn chinese_over_limit_truncated_on_char_boundary() {
        // Each Chinese char is 3 bytes in UTF-8. 30 chars = 90 bytes.
        let s = "评估外部攻击面共三十个字符的中文".repeat(2);
        let out = truncate_for_title(&s, 80);
        assert!(out.len() <= 80);
        // Must still be valid UTF-8 (no panic on indexing); chars() succeeds.
        let char_count = out.chars().count();
        assert!(char_count > 0);
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(truncate_for_title("", 80), "");
    }

    #[test]
    fn limit_zero_returns_empty() {
        assert_eq!(truncate_for_title("anything", 0), "");
    }

    #[test]
    fn task_mode_extracts_user_message_from_wrapped_prompt() {
        let prompt = "[System Context]\nlarge system prompt\n\n[User Message]\n帮我看看example.com";

        assert_eq!(
            extract_user_message_from_wrapped_prompt(prompt),
            "帮我看看example.com"
        );
    }

    #[test]
    fn task_mode_uses_plain_prompt_when_not_wrapped() {
        assert_eq!(
            extract_user_message_from_wrapped_prompt("帮我看看example.com"),
            "帮我看看example.com"
        );
    }

    #[test]
    fn triage_routes_greetings_to_conversation() {
        for greeting in [
            "你好",
            "  你好  ",
            "Hello",
            "hi there",
            "谢谢!",
            "你是谁",
            "你能做什么",
        ] {
            assert_eq!(
                deterministic_intent(greeting),
                Some(UserIntent::Conversation),
                "greeting must be conversational: {greeting:?}"
            );
        }
    }

    #[test]
    fn triage_routes_empty_to_conversation() {
        assert_eq!(deterministic_intent("   "), Some(UserIntent::Conversation));
    }

    #[test]
    fn triage_routes_clear_security_tasks_to_task() {
        for task in [
            "scan example.com for vulns",
            "对 https://target.tld 做渗透",
            "用 nmap 扫描 10.0.0.1",
            "enumerate subdomains of acme.io",
            "帮我做一次漏洞审计",
        ] {
            assert_eq!(
                deterministic_intent(task),
                Some(UserIntent::Task),
                "security task must be Task: {task:?}"
            );
        }
    }

    #[test]
    fn triage_task_signal_wins_over_greeting() {
        assert_eq!(
            deterministic_intent("你好，帮我扫描 example.com"),
            Some(UserIntent::Task)
        );
    }

    #[test]
    fn triage_defers_ambiguous_to_llm() {
        // No clear signal either way → caller falls back to the LLM classifier.
        assert_eq!(
            deterministic_intent("the auth module on the staging box"),
            None
        );
    }

    #[test]
    fn conversational_planner_failure_detects_soft_signals() {
        assert!(is_conversational_planner_failure(
            "Generator failed: The task planner declined to produce a plan — ..."
        ));
        assert!(is_conversational_planner_failure(
            "Generator failed: Failed to parse task planner JSON (missing field `subtasks` at line 3)"
        ));
        assert!(is_conversational_planner_failure(
            "[API trace=abc] send_ai_prompt_session: ... missing field `subtasks` ..."
        ));
    }

    #[test]
    fn conversational_planner_failure_ignores_real_errors() {
        assert!(!is_conversational_planner_failure(
            "Generator LLM call failed: connection refused"
        ));
        assert!(!is_conversational_planner_failure(
            "authentication failed (401)"
        ));
    }
}

/// Get vision capabilities for the current model in a session.
///
/// Returns information about whether the model supports images,
/// maximum image size, and supported formats.
#[tauri::command]
pub async fn get_vision_capabilities(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<golish_llm_providers::VisionCapabilities, GolishError> {
    let bridges = state.ai_state.get_bridges().await;
    let bridge = bridges
        .get(&session_id)
        .ok_or_else(|| super::super::ai_session_not_initialized_error(&session_id))?;

    Ok(golish_llm_providers::VisionCapabilities::detect(
        bridge.provider_name(),
        bridge.model_name(),
    ))
}

/// Send a multi-modal prompt (text + images) to the AI agent.
///
/// This command accepts a PromptPayload with multiple parts, enabling
/// image attachments for vision-capable models. If the model doesn't
/// support vision, images are stripped and a warning event is emitted.
///
/// IMPORTANT: Uses get_session_bridge() to clone the Arc and release the map
/// lock immediately. This allows other sessions to initialize/shutdown while
/// this session is executing, enabling true concurrent multi-tab agent execution.
#[tauri::command]
pub async fn send_ai_prompt_with_attachments(
    state: State<'_, AgentState>,
    session_id: String,
    payload: golish_core::PromptPayload,
) -> Result<String, GolishError> {
    use golish_core::PromptPart;
    use rig::message::{ImageMediaType, Text, UserContent};

    // Get Arc clone and release map lock immediately
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::super::ai_session_not_initialized_error(&session_id))?;

    // Check vision capabilities
    let caps = golish_llm_providers::VisionCapabilities::detect(
        bridge.provider_name(),
        bridge.model_name(),
    );

    // If provider doesn't support vision, strip images and emit warning
    let effective_payload = if payload.has_images() && !caps.supports_vision {
        tracing::warn!(
            "Provider {} doesn't support images, sending text-only",
            bridge.provider_name()
        );

        // Emit warning event to frontend
        bridge.emit_event(golish_core::AiEvent::Warning {
            message: format!(
                "Images removed: {} does not support vision",
                bridge.model_name()
            ),
        });

        golish_core::PromptPayload::from_text(payload.text_only())
    } else {
        // Validate payload
        payload.validate(caps.max_image_size_bytes, &caps.supported_formats)?;
        payload
    };

    // Convert PromptPayload to Vec<UserContent>
    let content_parts: Vec<UserContent> = effective_payload
        .parts
        .into_iter()
        .map(|p| match p {
            PromptPart::Text { text } => UserContent::Text(Text { text }),
            PromptPart::Image {
                data, media_type, ..
            } => {
                // Strip data URL prefix if present
                let has_data_url_prefix = data.starts_with("data:");
                let base64_data = if has_data_url_prefix {
                    data.split(',').nth(1).unwrap_or(&data).to_string()
                } else {
                    data
                };

                let img_media_type = media_type.as_deref().and_then(|mime| match mime {
                    "image/png" => Some(ImageMediaType::PNG),
                    "image/jpeg" | "image/jpg" => Some(ImageMediaType::JPEG),
                    "image/gif" => Some(ImageMediaType::GIF),
                    "image/webp" => Some(ImageMediaType::WEBP),
                    _ => None,
                });

                UserContent::image_base64(base64_data, img_media_type, None)
            }
        })
        .collect();

    // Execute without holding the map lock - other sessions can init/shutdown
    bridge
        .execute_with_content(content_parts)
        .await
        .map_err(GolishError::from)
}

/// Clear the conversation history for a specific session.
///
/// IMPORTANT: Uses get_session_bridge() to clone the Arc and release the map
/// lock immediately, avoiding deadlocks when other tasks need write access.
#[tauri::command]
pub async fn clear_ai_conversation_session(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::super::ai_session_not_initialized_error(&session_id))?;
    bridge.clear_conversation_history().await;
    tracing::info!("Conversation cleared for session {}", session_id);
    Ok(())
}

/// Get the conversation length for a specific session.
///
/// IMPORTANT: Uses get_session_bridge() to clone the Arc and release the map
/// lock immediately, avoiding deadlocks when other tasks need write access.
#[tauri::command]
pub async fn get_ai_conversation_length_session(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<usize, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::super::ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.conversation_history_len().await)
}

/// Signal that the frontend is ready to receive AI events for a session.
///
/// This command should be called by the frontend after it has set up its event listeners.
/// It causes any buffered events to be flushed to the frontend and enables direct event
/// emission going forward.
///
/// This solves race conditions where events are emitted before the frontend is ready
/// to receive them.
///
/// # Arguments
/// * `session_id` - The terminal session ID (tab) to signal ready for
#[tauri::command]
pub async fn signal_frontend_ready(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    tracing::info!(
        message = "[signal_frontend_ready] Frontend signaling ready",
        session_id = %session_id,
    );

    if let Some(bridge) = state.ai_state.get_session_bridge(&session_id).await {
        bridge.mark_frontend_ready().await;
        tracing::debug!(
            message = "[signal_frontend_ready] Marked frontend as ready",
            session_id = %session_id,
        );
    } else {
        tracing::debug!(
            message = "[signal_frontend_ready] No bridge found for session (may not be initialized yet)",
            session_id = %session_id,
        );
    }

    Ok(())
}
