//! AI chat commands: send prompt (with attachments), clear conv, signal ready,
//! cancel, vision capabilities.

use std::sync::Arc;

use tauri::State;

use super::super::super::agent_bridge::AgentBridge;
use crate::state::AppState;


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
    state: State<'_, AppState>,
    session_id: String,
    prompt: String,
) -> Result<String, String> {
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
        golish_ai::execution_mode::ExecutionMode::Chat => {
            bridge.execute(&prompt).await.map_err(|e| {
                tracing::error!(
                    message = "[send_ai_prompt_session] Chat execution error",
                    session_id = %session_id,
                    error = %e,
                );
                e.to_string()
            })
        }
        golish_ai::execution_mode::ExecutionMode::Task => {
            use golish_ai::task_orchestrator::bridge_executor::{classify_user_intent, UserIntent};

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
                    e.to_string()
                })
            } else {
                execute_task_mode(bridge, &session_id, &prompt)
                    .await
                    .map_err(|e| {
                        tracing::error!(
                            message = "[send_ai_prompt_session] Task execution error",
                            session_id = %session_id,
                            error = %e,
                        );
                        e.to_string()
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
) -> anyhow::Result<String> {
    use golish_ai::task_orchestrator::{bridge_executor::BridgeAgentExecutor, TaskOrchestrator};
    use golish_core::events::AiEvent;

    let pool = bridge
        .db_pool()
        .ok_or_else(|| anyhow::anyhow!("Database pool not available — Task mode requires a DB connection"))?;

    let db_session = golish_db::repo::sessions::create(
        &pool,
        golish_db::models::NewSession {
            title: Some(format!("Task: {}", &prompt[..prompt.len().min(50)])),
            workspace_path: None,
            workspace_label: None,
            model: Some(bridge.model_name().to_string()),
            provider: Some(bridge.provider_name().to_string()),
            project_path: None,
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("Failed to create DB session for task mode: {}", e))?;
    let uuid_session_id = db_session.id;

    let event_tx = bridge.get_or_create_event_tx();

    // Emit a short initial message so the user sees immediate feedback.
    // This cycle completes before the Generator call so it won't collide
    // with per-subtask Started/Completed events from execute_isolated.
    let init_turn = uuid::Uuid::new_v4().to_string();
    let init_msg = format!(
        "I'm analyzing your request and generating a task plan. \
         This may take a moment while I decompose the task into subtasks..."
    );
    bridge.emit_event(AiEvent::Started { turn_id: init_turn });
    bridge.emit_event(AiEvent::UserMessage { content: prompt.to_string() });
    bridge.emit_event(AiEvent::TextDelta {
        delta: init_msg.clone(),
        accumulated: init_msg.clone(),
    });
    bridge.emit_event(AiEvent::Completed {
        response: init_msg,
        reasoning: None,
        input_tokens: None,
        output_tokens: None,
        duration_ms: Some(0),
    });

    let start_time = std::time::Instant::now();
    let mut orchestrator = TaskOrchestrator::new(pool, uuid_session_id, event_tx);
    let executor = BridgeAgentExecutor::new(bridge.clone());

    let result = orchestrator.run(prompt, &executor).await;

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
            bridge.emit_event(AiEvent::Started { turn_id: report_turn });
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
                message: e.to_string(),
                error_type: "task_orchestrator".to_string(),
            });
        }
    }

    result
}

/// Get vision capabilities for the current model in a session.
///
/// Returns information about whether the model supports images,
/// maximum image size, and supported formats.
#[tauri::command]
pub async fn get_vision_capabilities(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<golish_llm_providers::VisionCapabilities, String> {
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
    state: State<'_, AppState>,
    session_id: String,
    payload: golish_core::PromptPayload,
) -> Result<String, String> {
    use golish_core::PromptPart;
    use rig::message::{ImageMediaType, Text, UserContent};

    // Get Arc clone and release map lock immediately
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::super::ai_session_not_initialized_error(&session_id))?;

    // Check vision capabilities
    let caps =
        golish_llm_providers::VisionCapabilities::detect(bridge.provider_name(), bridge.model_name());

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
        .map_err(|e| e.to_string())
}

/// Clear the conversation history for a specific session.
///
/// IMPORTANT: Uses get_session_bridge() to clone the Arc and release the map
/// lock immediately, avoiding deadlocks when other tasks need write access.
#[tauri::command]
pub async fn clear_ai_conversation_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
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
    state: State<'_, AppState>,
    session_id: String,
) -> Result<usize, String> {
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
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
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
