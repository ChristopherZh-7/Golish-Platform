//! Build the per-iteration `CompletionRequest` and start a streaming response,
//! with bounded exponential-backoff retries on transient failures.
//!
//! Provider-specific quirks handled here:
//!
//! - **OpenAI web search**: enabled via `additional_params.tools` when
//!   `ctx.llm.openai_web_search_config` is set.
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
use rig::message::{ToolChoice, UserContent};
use rig::one_or_many::OneOrMany;
use rig::streaming::StreamingCompletionResponse;
use serde_json::json;
use tracing::Instrument;

use golish_core::events::AiEvent;
use golish_llm_providers::{resolve_stream_quirks, ThinkingDisableField};

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
const CANCEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// Build a `CompletionRequest` from the current iteration's state and start a
/// streaming response, retrying transient failures up to
/// [`STREAM_START_MAX_ATTEMPTS`] times.
///
/// On a non-retriable failure (or after exhausting retries) this emits a
/// terminal `AiEvent::Error` and returns a [`TerminalErrorEmitted`] with the
/// supplied `accumulated_response` and a clone of `chat_history` attached so
/// the bridge can persist partial state.
pub(crate) async fn start_completion_stream<M>(
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
            ctx.llm.model_name
        );
        None
    };

    let additional_params = build_additional_params(ctx);

    // NVIDIA NIM workaround: see module docs.
    let is_nvidia_provider = ctx.llm.provider_name == "nvidia";
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

    // Force native tool calls for the autonomous depth-0 primary inside a
    // harness stage on providers that otherwise narrate tool calls as
    // text/XML or empty args under load (see `resolve_stage_tool_choice`).
    let tool_choice = resolve_stage_tool_choice(
        ctx.llm.provider_name,
        ctx.harness_stage.is_some(),
        config.is_sub_agent,
        !request_tools.is_empty(),
    );
    if tool_choice.is_some() {
        tracing::info!(
            provider = ctx.llm.provider_name,
            harness_stage = ctx.harness_stage.is_some(),
            "[tool-choice] Forcing tool_choice=required for harness-stage primary"
        );
    }

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
            tool_choice: tool_choice.clone(),
            additional_params: additional_params.clone(),
            model: None,
            output_schema: None,
        };

        if is_cancelled(ctx) {
            tracing::info!("Agent cancelled before LLM call (attempt {})", attempt);
            let _ = ctx.events.event_tx.send(AiEvent::Error {
                message: "Agent stopped by user".to_string(),
                error_type: "cancelled".to_string(),
            });
            return Err(anyhow::anyhow!("Agent stopped by user"));
        }

        ctx.api_request_stats
            .record_sent(ctx.llm.provider_name)
            .await;

        let stream_result = tokio::select! {
            biased;
            _ = wait_for_cancelled(ctx) => {
                tracing::info!("Agent cancelled while starting LLM stream (attempt {})", attempt);
                let _ = ctx.events.event_tx.send(AiEvent::Error {
                    message: "Agent stopped by user".to_string(),
                    error_type: "cancelled".to_string(),
                });
                return Err(TerminalErrorEmitted::with_partial_state(
                    "Agent stopped by user",
                    (!accumulated_response.is_empty()).then(|| accumulated_response.to_string()),
                    Some(chat_history.to_vec()),
                )
                .into());
            }
            result = tokio::time::timeout(
                STREAM_START_TIMEOUT,
                async { model.stream(request).await }.instrument(llm_span.clone()),
            ) => result,
        };

        match stream_result {
            Ok(Ok(s)) => {
                ctx.api_request_stats
                    .record_received(ctx.llm.provider_name)
                    .await;
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
                    let _ = ctx.events.event_tx.send(AiEvent::Warning {
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
                    let _ = ctx.events.event_tx.send(AiEvent::Warning {
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

    let _ = ctx.events.event_tx.send(AiEvent::Error {
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

    if let Some(web_config) = ctx.llm.openai_web_search_config {
        tracing::info!(
            "Adding OpenAI web_search_preview tool with context_size={}",
            web_config.search_context_size
        );
        additional_params_json.insert("tools".to_string(), json!([web_config.to_tool_json()]));
    }

    // OpenAI Responses API expects a nested `reasoning` object with:
    // - effort: how much thinking the model should do
    // - summary: enables streaming reasoning text to the client
    //   ("detailed" shows full reasoning)
    if let Some(effort) = ctx.llm.openai_reasoning_effort {
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

    if let Some(serde_json::Value::Object(prefs_map)) = ctx.llm.openrouter_provider_preferences {
        for (key, value) in prefs_map {
            tracing::info!("Adding OpenRouter provider preference: {}={}", key, value);
            additional_params_json.insert(key.clone(), value.clone());
        }
    }

    let quirks = resolve_stream_quirks(
        ctx.llm.provider_name,
        ctx.llm.model_name,
        ctx.llm.model_override,
    );
    if let Some(thinking_value) = quirks.thinking_kwargs_value {
        match quirks.disable_thinking_field {
            ThinkingDisableField::ChatTemplateKwargs => {
                tracing::info!(
                    "[Quirks] Injecting chat_template_kwargs.enable_thinking={} for {} / {}",
                    thinking_value,
                    ctx.llm.provider_name,
                    ctx.llm.model_name
                );
                let existing = additional_params_json
                    .entry("chat_template_kwargs".to_string())
                    .or_insert_with(|| json!({}));
                if let Some(obj) = existing.as_object_mut() {
                    obj.insert("enable_thinking".to_string(), json!(thinking_value));
                }
            }
            ThinkingDisableField::TopLevelEnableThinking => {
                tracing::info!(
                    "[Quirks] Injecting top-level enable_thinking={} for {} / {}",
                    thinking_value,
                    ctx.llm.provider_name,
                    ctx.llm.model_name
                );
                additional_params_json.insert("enable_thinking".to_string(), json!(thinking_value));
            }
            ThinkingDisableField::OpenRouterReasoningExclude => {
                // OpenRouter inverts the convention: `reasoning.exclude=true`
                // means *don't* return reasoning; we map our positive value
                // accordingly.
                let exclude = !thinking_value;
                tracing::info!(
                    "[Quirks] Injecting reasoning.exclude={} for {} / {}",
                    exclude,
                    ctx.llm.provider_name,
                    ctx.llm.model_name
                );
                let existing = additional_params_json
                    .entry("reasoning".to_string())
                    .or_insert_with(|| json!({}));
                if let Some(obj) = existing.as_object_mut() {
                    obj.insert("exclude".to_string(), json!(exclude));
                }
            }
            ThinkingDisableField::None => {
                tracing::warn!(
                    "[Quirks] thinking_kwargs_value set but no disable field configured for {} / {}",
                    ctx.llm.provider_name,
                    ctx.llm.model_name
                );
            }
        }
    }

    if additional_params_json.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(additional_params_json))
    }
}

/// Providers whose OpenAI/Anthropic-compatible endpoints accept
/// `tool_choice=required` and emit cleaner native tool calls when forced.
///
/// Xiaomi MiMo narrates tool calls as textual XML (OpenAI protocol) or as
/// `tool_use` blocks with empty arguments (Anthropic protocol) once the tool
/// surface + context grow large; a raw-SSE probe (2026-06-07) confirmed both
/// endpoints accept `tool_choice=required` and then stream native calls with
/// populated arguments. Listed here so the runtime forces native tool use only
/// for the providers that need it.
const FORCE_REQUIRED_TOOL_CHOICE_PROVIDERS: &[&str] = &["xiaomi"];

/// Decide the `tool_choice` for this turn.
///
/// Returns `Some(ToolChoice::Required)` only for the autonomous **depth-0
/// primary running inside a harness stage** on a provider in
/// [`FORCE_REQUIRED_TOOL_CHOICE_PROVIDERS`]. Inside a harness stage the primary
/// has no legitimate text-only turn — every action is a tool call (`update_plan`
/// / `sub_agent_*` / `manage_*` / `ask_human` / `submit_stage_deliverable`), so
/// forcing native tool calls removes the "prose/XML instead of a tool call"
/// failure without changing behavior anywhere else:
///
/// - chat (no harness stage) keeps provider-default `auto`, preserving the
///   loop's text-only termination path;
/// - sub-agents (`is_sub_agent`) keep `auto` so they can return a final text
///   summary;
/// - reliable native providers are untouched.
fn resolve_stage_tool_choice(
    provider_name: &str,
    in_harness_stage: bool,
    is_sub_agent: bool,
    has_tools: bool,
) -> Option<ToolChoice> {
    if in_harness_stage
        && !is_sub_agent
        && has_tools
        && FORCE_REQUIRED_TOOL_CHOICE_PROVIDERS.contains(&provider_name)
    {
        Some(ToolChoice::Required)
    } else {
        None
    }
}

async fn wait_for_cancelled(ctx: &AgenticLoopContext<'_>) {
    loop {
        if is_cancelled(ctx) {
            return;
        }
        tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_stage_tool_choice, ToolChoice};

    #[test]
    fn forces_required_for_xiaomi_primary_in_harness_stage_with_tools() {
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", true, false, true),
            Some(ToolChoice::Required)
        );
    }

    #[test]
    fn no_force_outside_harness_stage() {
        // Chat / non-harness task turns keep provider-default `auto`.
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", false, false, true),
            None
        );
    }

    #[test]
    fn no_force_for_sub_agent() {
        // Sub-agents must be able to end on a text-only summary turn.
        assert_eq!(resolve_stage_tool_choice("xiaomi", true, true, true), None);
    }

    #[test]
    fn no_force_without_tools() {
        // Forcing a tool call with no tools available would be unsatisfiable.
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", true, false, false),
            None
        );
    }

    #[test]
    fn no_force_for_reliable_providers() {
        // Reliable native providers keep their existing behavior unchanged.
        assert_eq!(
            resolve_stage_tool_choice("anthropic", true, false, true),
            None
        );
        assert_eq!(resolve_stage_tool_choice("openai", true, false, true), None);
        assert_eq!(resolve_stage_tool_choice("nvidia", true, false, true), None);
    }
}
