//! AI chat commands: send prompt (with attachments), clear conv, signal ready,
//! cancel, vision capabilities.

use crate::error::GolishError;
use std::sync::Arc;

use tauri::State;

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
            use golish_agent_bridge::bridge_executor::{classify_user_intent, UserIntent};

            let intent = classify_user_intent(&bridge, &prompt).await;

            if intent == UserIntent::Conversation {
                tracing::info!(
                    message = "[send_ai_prompt_session] Task mode but conversational intent — using Chat path",
                    session_id = %session_id,
                );
                bridge.execute(&prompt).await.map_err(|e| {
                    tracing::error!(
                        message = "[send_ai_prompt_session] Chat-fallback execution error",
                        session_id = %session_id,
                        error = %e,
                    );
                    GolishError::Internal(e.to_string())
                })
            } else {
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

    // Lazy-create a `sessions` row so that `tasks.session_id` FK
    // (UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE) is satisfied.
    // The chat-panel-level string session id (`_session_id`) is not a UUID and
    // is not currently mirrored into the `sessions` table; mapping it back
    // requires a separate schema change. Phase 1 keeps task DB rows isolated
    // per task invocation; future work can dedupe by chat session.
    let session_row = sessions::create(
        &state.db_pool,
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
    .context("Failed to create session row for task mode (FK precondition for tasks)")?;
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

    let result = orchestrator.run(task_input, &executor).await;

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
            bridge.emit_event(AiEvent::Error {
                message: format!("{:#}", e),
                error_type: "task_orchestrator".to_string(),
            });
        }
    }

    result
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
    use super::{extract_user_message_from_wrapped_prompt, truncate_for_title};

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
