//! Build the per-iteration `CompletionRequest` and start a streaming response,
//! with bounded exponential-backoff retries on transient failures.
//!
//! Provider-specific quirks handled here:
//!
//! - **OpenAI web search**: enabled via `additional_params.tools` when
//!   `ctx.openai_web_search_config` is set.
//! - **OpenAI reasoning** (`o-series`, `gpt-5.2 Codex`): nested `reasoning`
//!   object with `effort` + `summary` keys.
//! - **OpenRouter provider preferences**: forwarded as top-level keys in
//!   `additional_params`.
//! - **NVIDIA NIM**: rig-core's OpenAI provider serializes the system message
//!   as an array of `{type, text}` objects, but NVIDIA NIM only accepts plain
//!   strings — so we move the system prompt into a leading user message and
//!   rely on rig-core's user-content flattener instead of `preamble`.

use anyhow::Result;
use rig::completion::Message;
use rig::message::UserContent;
use rig::one_or_many::OneOrMany;
use rig::streaming::StreamingCompletionResponse;
use serde_json::json;
use tracing::Instrument;

use golish_core::events::AiEvent;

use super::config::AgenticLoopConfig;
use super::context::{is_cancelled, AgenticLoopContext, TerminalErrorEmitted};
use super::stream_retry::{
    classify_stream_start_error, compute_retry_backoff_delay, should_retry_stream_start,
    sleep_for_retry_delay, stream_start_timeout_classification, StreamStartErrorClassification,
    STREAM_START_MAX_ATTEMPTS,
};
use super::MAX_COMPLETION_TOKENS;

/// Wrap stream startup with a 3 minute timeout to prevent infinite hangs.
const STREAM_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// Build a `CompletionRequest` from the current iteration's state and start a
/// streaming response, retrying transient failures up to
/// [`STREAM_START_MAX_ATTEMPTS`] times.
///
/// On a non-retriable failure (or after exhausting retries) this emits a
/// terminal `AiEvent::Error` and returns a [`TerminalErrorEmitted`] with the
/// supplied `accumulated_response` and a clone of `chat_history` attached so
/// the bridge can persist partial state.
pub(super) async fn start_completion_stream<M>(
    ctx: &AgenticLoopContext<'_>,
    config: &AgenticLoopConfig,
    model: &M,
    system_prompt: &str,
    chat_history: &[Message],
    tools: &[rig::completion::ToolDefinition],
    llm_span: &tracing::Span,
    accumulated_response: &str,
) -> Result<StreamingCompletionResponse<M::StreamingResponse>>
where
    M: rig::completion::CompletionModel + Sync,
{
    let temperature = if config.capabilities.supports_temperature {
        Some(0.3)
    } else {
        tracing::debug!(
            "Model {} does not support temperature parameter, omitting",
            ctx.model_name
        );
        None
    };

    let additional_params = build_additional_params(ctx);

    // NVIDIA NIM workaround: see module docs.
    let is_nvidia_provider = ctx.provider_name == "nvidia";
    let (preamble, request_history) = if is_nvidia_provider {
        let mut nvidia_history = vec![Message::User {
            content: OneOrMany::one(UserContent::text(system_prompt)),
        }];
        nvidia_history.extend(chat_history.iter().cloned());
        (None, nvidia_history)
    } else {
        (Some(system_prompt.to_string()), chat_history.to_vec())
    };
    let request_chat_history = OneOrMany::many(request_history.clone())
        .unwrap_or_else(|_| OneOrMany::one(request_history[0].clone()));
    let request_tools = tools.to_vec();

    let mut stream_start_failure: Option<(String, StreamStartErrorClassification)> = None;
    let mut started_stream = None;

    for attempt in 1..=STREAM_START_MAX_ATTEMPTS {
        let request = rig::completion::CompletionRequest {
            preamble: preamble.clone(),
            chat_history: request_chat_history.clone(),
            documents: vec![],
            tools: request_tools.clone(),
            temperature,
            max_tokens: Some(MAX_COMPLETION_TOKENS as u64),
            tool_choice: None,
            additional_params: additional_params.clone(),
            model: None,
            output_schema: None,
        };

        if is_cancelled(ctx) {
            tracing::info!("Agent cancelled before LLM call (attempt {})", attempt);
            let _ = ctx.event_tx.send(AiEvent::Error {
                message: "Agent stopped by user".to_string(),
                error_type: "cancelled".to_string(),
            });
            return Err(anyhow::anyhow!("Agent stopped by user"));
        }

        ctx.api_request_stats.record_sent(ctx.provider_name).await;

        let stream_result = tokio::time::timeout(
            STREAM_START_TIMEOUT,
            async { model.stream(request).await }.instrument(llm_span.clone()),
        )
        .await;

        match stream_result {
            Ok(Ok(s)) => {
                ctx.api_request_stats.record_received(ctx.provider_name).await;
                tracing::info!(
                    "[OpenAI Debug] Stream created successfully on attempt {}",
                    attempt
                );
                started_stream = Some(s);
                break;
            }
            Ok(Err(e)) => {
                let error_str = e.to_string();
                let classification = classify_stream_start_error(&error_str);
                tracing::warn!(
                    "Stream start failed (attempt {}/{}): {}",
                    attempt,
                    STREAM_START_MAX_ATTEMPTS,
                    error_str
                );

                if should_retry_stream_start(attempt, &classification) {
                    let delay = compute_retry_backoff_delay(attempt);
                    let delay_ms = delay.as_millis();
                    let _ = ctx.event_tx.send(AiEvent::Warning {
                        message: format!(
                            "AI request failed ({}). Retrying in {}ms (attempt {}/{})",
                            classification.error_type,
                            delay_ms,
                            attempt + 1,
                            STREAM_START_MAX_ATTEMPTS
                        ),
                    });
                    sleep_for_retry_delay(delay).await;
                    continue;
                }

                stream_start_failure = Some((error_str, classification));
                break;
            }
            Err(_elapsed) => {
                let timeout_secs = STREAM_START_TIMEOUT.as_secs();
                let error_str = format!("Stream request timeout after {}s", timeout_secs);
                let classification = stream_start_timeout_classification(timeout_secs);
                tracing::warn!(
                    "[OpenAI Debug] Stream request timed out (attempt {}/{}): {}",
                    attempt,
                    STREAM_START_MAX_ATTEMPTS,
                    error_str
                );

                if should_retry_stream_start(attempt, &classification) {
                    let delay = compute_retry_backoff_delay(attempt);
                    let delay_ms = delay.as_millis();
                    let _ = ctx.event_tx.send(AiEvent::Warning {
                        message: format!(
                            "AI request timed out. Retrying in {}ms (attempt {}/{})",
                            delay_ms,
                            attempt + 1,
                            STREAM_START_MAX_ATTEMPTS
                        ),
                    });
                    sleep_for_retry_delay(delay).await;
                    continue;
                }

                stream_start_failure = Some((error_str, classification));
                break;
            }
        }
    }

    if let Some(stream) = started_stream {
        return Ok(stream);
    }

    let (error_str, classification) = stream_start_failure.unwrap_or_else(|| {
        (
            "Failed to start streaming response".to_string(),
            StreamStartErrorClassification {
                error_type: "api_error",
                user_message: "Failed to start streaming response".to_string(),
                retriable: false,
            },
        )
    });

    let _ = ctx.event_tx.send(AiEvent::Error {
        message: classification.user_message,
        error_type: classification.error_type.to_string(),
    });

    Err(TerminalErrorEmitted::with_partial_state(
        error_str,
        (!accumulated_response.is_empty()).then(|| accumulated_response.to_string()),
        Some(chat_history.to_vec()),
    )
    .into())
}

/// Assemble the optional `additional_params` JSON object from provider-specific
/// `ctx` fields.
fn build_additional_params(ctx: &AgenticLoopContext<'_>) -> Option<serde_json::Value> {
    let mut additional_params_json = serde_json::Map::new();

    if let Some(web_config) = ctx.openai_web_search_config {
        tracing::info!(
            "Adding OpenAI web_search_preview tool with context_size={}",
            web_config.search_context_size
        );
        additional_params_json
            .insert("tools".to_string(), json!([web_config.to_tool_json()]));
    }

    // OpenAI Responses API expects a nested `reasoning` object with:
    // - effort: how much thinking the model should do
    // - summary: enables streaming reasoning text to the client
    //   ("detailed" shows full reasoning)
    if let Some(effort) = ctx.openai_reasoning_effort {
        tracing::info!(
            "Setting OpenAI reasoning.effort={}, reasoning.summary=detailed",
            effort
        );
        additional_params_json.insert(
            "reasoning".to_string(),
            json!({
                "effort": effort,
                "summary": "detailed"
            }),
        );
    }

    if let Some(prefs) = ctx.openrouter_provider_preferences {
        if let serde_json::Value::Object(prefs_map) = prefs {
            for (key, value) in prefs_map {
                tracing::info!("Adding OpenRouter provider preference: {}={}", key, value);
                additional_params_json.insert(key.clone(), value.clone());
            }
        }
    }

    if additional_params_json.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(additional_params_json))
    }
}
