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
    submit_only: bool,
    forced_tool: Option<&str>,
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

    let mut additional_params = build_additional_params(ctx);
    let request_tools = tools.to_vec();
    let submit_tool_available = request_tools
        .iter()
        .any(|t| t.name == SUBMIT_STAGE_DELIVERABLE_TOOL);
    let forced_tool = forced_tool.filter(|tool| request_tools.iter().any(|t| t.name == *tool));

    // 设计 2026-06-12 (防御 C) · submit-only 轮把「只准提交」硬约束写进模型可见的
    // 系统提示（API 层 tool_choice 对忽略它的 provider 无效时兜底）。
    let effective_system_prompt = compose_system_prompt(system_prompt, submit_only, forced_tool);

    // NVIDIA NIM workaround: see module docs.
    let is_nvidia_provider = ctx.llm.provider_name == "nvidia";
    let (preamble, request_history) = if is_nvidia_provider {
        let mut nvidia_history = vec![Message::User {
            content: OneOrMany::one(UserContent::text(effective_system_prompt)),
        }];
        nvidia_history.extend(chat_history.iter().cloned());
        (None, nvidia_history)
    } else {
        (Some(effective_system_prompt), chat_history.to_vec())
    };
    let request_chat_history = OneOrMany::many(request_history.clone())
        .unwrap_or_else(|_| OneOrMany::one(request_history[0].clone()));

    // Force native tool calls for the autonomous depth-0 primary inside a
    // harness stage on providers that otherwise narrate tool calls as
    // text/XML or empty args under load (see `resolve_stage_tool_choice`).
    // 设计 2026-06-11 · on a targeted gate-repair pass (`submit_only`) the
    // choice tightens further to the specific `submit_stage_deliverable` tool.
    // 2026-06-28 · bare resume can also force a known orchestration tool
    // (`stage_run`) for the first resumed iteration, bypassing context-probing
    // tools while still using the normal runtime/gate path.
    let native_tool_choice_allowed = !request_uses_tool_choice_incompatible_thinking_mode(
        ctx.llm.provider_name,
        ctx.llm.model_name,
        config.capabilities.supports_thinking_history,
        ctx.llm.openai_reasoning_effort.is_some(),
        additional_params.as_ref(),
    );
    if !native_tool_choice_allowed {
        tracing::info!(
            provider = ctx.llm.provider_name,
            model = ctx.llm.model_name,
            "[tool-choice] Suppressing native tool_choice because thinking mode is active"
        );
    }
    let tool_choice = resolve_stage_tool_choice(
        ctx.llm.provider_name,
        ctx.harness_stage.is_some(),
        config.is_sub_agent,
        !request_tools.is_empty(),
        submit_only && submit_tool_available,
        forced_tool,
        native_tool_choice_allowed,
    );
    // rig-core's OpenAI Chat provider hard-errors on `ToolChoice::Specific`,
    // while its Anthropic provider converts it natively. For OpenAI-protocol
    // clients, route the named choice through `additional_params` (serde-flatten
    // merges it top-level into the request body) in OpenAI wire format instead.
    // Live-probed 2026-06-11: both Xiaomi endpoints accept their respective
    // named tool_choice wire formats and answer with exactly the named tool.
    let specific_tool_name = match &tool_choice {
        Some(ToolChoice::Specific { function_names }) => function_names.first().cloned(),
        _ => None,
    };
    let client_is_anthropic = ctx.llm.client.read().await.is_anthropic();
    let mut tool_choice =
        if let Some(tool_name) = specific_tool_name.filter(|_| !client_is_anthropic) {
            let named = json!({
                "tool_choice": {
                    "type": "function",
                    "function": { "name": tool_name }
                }
            });
            additional_params = Some(match additional_params.take() {
                Some(serde_json::Value::Object(mut obj)) => {
                    obj.extend(named.as_object().cloned().unwrap_or_default());
                    serde_json::Value::Object(obj)
                }
                _ => named,
            });
            None
        } else {
            tool_choice
        };
    if tool_choice.is_some() || submit_only || forced_tool.is_some() {
        tracing::info!(
            provider = ctx.llm.provider_name,
            harness_stage = ctx.harness_stage.is_some(),
            submit_only,
            forced_tool,
            submit_tool_available,
            native_tool_choice_allowed,
            choice = ?tool_choice,
            "[tool-choice] Forcing tool_choice for harness-stage primary"
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

                if tool_choice_rejected_by_thinking_mode(&error_str)
                    && (tool_choice.is_some()
                        || additional_params_has_tool_choice(&additional_params))
                {
                    tracing::warn!(
                        provider = ctx.llm.provider_name,
                        model = ctx.llm.model_name,
                        "[tool-choice] Provider rejected tool_choice in thinking mode; retrying without native tool_choice"
                    );
                    tool_choice = None;
                    strip_tool_choice_from_additional_params(&mut additional_params);
                    continue;
                }

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

/// The stage-submission tool a targeted gate-repair pass locks onto.
pub(crate) const SUBMIT_STAGE_DELIVERABLE_TOOL: &str = "submit_stage_deliverable";

/// 设计 2026-06-12 (submit-only-lock-hardening 防御 C) · prompt 级硬约束。
/// API 层 `tool_choice` 对忽略它的 provider（实测 xiaomi/MiMo）形同虚设，所以
/// 在 submit-only 轮把「只准提交」的指令写进模型看得见的系统提示里兜底。对遵从
/// `tool_choice` 的 provider 是无害冗余。
const SUBMIT_ONLY_PROMPT_DIRECTIVE: &str = "\n\n## MANDATORY — SUBMIT-ONLY TURN\n\
     This stage's scan work is ALREADY complete and its evidence is recorded in the ledger. \
     Your ENTIRE next response MUST be a single `submit_stage_deliverable` tool call. \
     Do NOT run tools, do NOT delegate to sub-agents, do NOT call update_plan, and do NOT \
     write analysis, plans, or narration. Cite ONLY the real evidence ids already provided. \
     Emit the `submit_stage_deliverable` call now — nothing else.";

fn forced_tool_prompt_directive(tool_name: &str) -> String {
    let args_hint = if tool_name == "stage_run" {
        " Use arguments {\"orgs\":[]} so the runtime expands the bound engagement \
         organization subtree from the database."
    } else {
        " Use the minimal valid arguments for that tool."
    };
    format!(
        "\n\n## MANDATORY — TOOL-LOCKED TURN\n\
         The runtime is resuming a checkpoint and already knows the next action. \
         Your ENTIRE next response MUST be a single `{tool_name}` tool call.{args_hint} \
         Do NOT call update_plan, read/list/query tools, or write analysis, plans, or narration. \
         Emit `{tool_name}` now — nothing else."
    )
}

/// 设计 2026-06-12 · submit-only 轮在系统提示末尾追加 [`SUBMIT_ONLY_PROMPT_DIRECTIVE`]；
/// 否则原样返回。纯函数，便于单测。
pub(crate) fn compose_system_prompt(
    system_prompt: &str,
    submit_only: bool,
    forced_tool: Option<&str>,
) -> String {
    if submit_only {
        format!("{system_prompt}{SUBMIT_ONLY_PROMPT_DIRECTIVE}")
    } else if let Some(tool_name) = forced_tool {
        format!("{system_prompt}{}", forced_tool_prompt_directive(tool_name))
    } else {
        system_prompt.to_string()
    }
}

/// Decide the `tool_choice` for this turn.
///
/// Returns `Some` only for the autonomous **depth-0 primary running inside a
/// harness stage** on a provider in [`FORCE_REQUIRED_TOOL_CHOICE_PROVIDERS`].
/// Inside a harness stage the primary has no legitimate text-only turn — every
/// action is a tool call (`update_plan` / `sub_agent_*` / `manage_*` /
/// `ask_human` / `submit_stage_deliverable`), so forcing native tool calls
/// removes the "prose/XML instead of a tool call" failure without changing
/// behavior anywhere else:
///
/// - chat (no harness stage) keeps provider-default `auto`, preserving the
///   loop's text-only termination path;
/// - sub-agents (`is_sub_agent`) keep `auto` so they can return a final text
///   summary;
/// - reliable native providers are untouched.
///
/// 设计 2026-06-11 (weak-model-submit-channel) · `submit_only=true` (a targeted
/// gate-repair pass whose sole remaining action is the stage submission, with
/// the submit tool present and not yet dispatched) tightens the choice from
/// `Required` to the specific `submit_stage_deliverable` tool, structurally
/// preventing a weak model from redoing the stage instead of submitting.
fn resolve_stage_tool_choice(
    provider_name: &str,
    in_harness_stage: bool,
    is_sub_agent: bool,
    has_tools: bool,
    submit_only: bool,
    forced_tool: Option<&str>,
    native_tool_choice_allowed: bool,
) -> Option<ToolChoice> {
    if !(native_tool_choice_allowed && in_harness_stage && !is_sub_agent && has_tools) {
        return None;
    }

    if let Some(tool_name) = forced_tool {
        return Some(ToolChoice::Specific {
            function_names: vec![tool_name.to_string()],
        });
    }

    if FORCE_REQUIRED_TOOL_CHOICE_PROVIDERS.contains(&provider_name) {
        if submit_only {
            Some(ToolChoice::Specific {
                function_names: vec![SUBMIT_STAGE_DELIVERABLE_TOOL.to_string()],
            })
        } else {
            Some(ToolChoice::Required)
        }
    } else {
        None
    }
}

fn request_uses_tool_choice_incompatible_thinking_mode(
    provider_name: &str,
    model_name: &str,
    supports_thinking_history: bool,
    openai_reasoning_effort_set: bool,
    additional_params: Option<&serde_json::Value>,
) -> bool {
    if request_explicitly_enables_thinking(openai_reasoning_effort_set, additional_params) {
        return true;
    }

    provider_defaults_to_tool_choice_incompatible_thinking(
        provider_name,
        model_name,
        supports_thinking_history,
    )
}

fn provider_defaults_to_tool_choice_incompatible_thinking(
    provider_name: &str,
    model_name: &str,
    supports_thinking_history: bool,
) -> bool {
    if !supports_thinking_history {
        return false;
    }

    match provider_name {
        "deepseek" => !model_name.to_lowercase().ends_with("deepseek-chat"),
        "openai_reasoning" | "openai_responses" => true,
        "openai" => golish_llm_providers::is_reasoning_model(model_name),
        _ => false,
    }
}

fn request_explicitly_enables_thinking(
    openai_reasoning_effort_set: bool,
    additional_params: Option<&serde_json::Value>,
) -> bool {
    if openai_reasoning_effort_set {
        return true;
    }

    let Some(serde_json::Value::Object(params)) = additional_params else {
        return false;
    };

    if matches!(
        params.get("enable_thinking"),
        Some(serde_json::Value::Bool(true))
    ) {
        return true;
    }

    if matches!(
        params
            .get("chat_template_kwargs")
            .and_then(serde_json::Value::as_object)
            .and_then(|obj| obj.get("enable_thinking")),
        Some(serde_json::Value::Bool(true))
    ) {
        return true;
    }

    if let Some(reasoning) = params
        .get("reasoning")
        .and_then(serde_json::Value::as_object)
    {
        if matches!(
            reasoning.get("exclude"),
            Some(serde_json::Value::Bool(true))
        ) {
            return false;
        }
        return true;
    }

    false
}

fn tool_choice_rejected_by_thinking_mode(error: &str) -> bool {
    let err = error.to_lowercase();
    err.contains("tool_choice")
        && (err.contains("thinking") || err.contains("reasoning"))
        && (err.contains("does not support")
            || err.contains("not support")
            || err.contains("unsupported")
            || err.contains("invalid_request"))
}

fn additional_params_has_tool_choice(additional_params: &Option<serde_json::Value>) -> bool {
    matches!(
        additional_params,
        Some(serde_json::Value::Object(obj)) if obj.contains_key("tool_choice")
    )
}

fn strip_tool_choice_from_additional_params(additional_params: &mut Option<serde_json::Value>) {
    if let Some(serde_json::Value::Object(obj)) = additional_params {
        obj.remove("tool_choice");
        if obj.is_empty() {
            *additional_params = None;
        }
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
    use super::{
        additional_params_has_tool_choice, compose_system_prompt,
        request_uses_tool_choice_incompatible_thinking_mode, resolve_stage_tool_choice,
        strip_tool_choice_from_additional_params, tool_choice_rejected_by_thinking_mode,
        ToolChoice, SUBMIT_STAGE_DELIVERABLE_TOOL,
    };
    use serde_json::json;

    // 设计 2026-06-12 (防御 C) · prompt 级硬约束注入。
    #[test]
    fn compose_system_prompt_is_identity_when_not_submit_only() {
        assert_eq!(
            compose_system_prompt("base prompt", false, None),
            "base prompt"
        );
    }

    #[test]
    fn compose_system_prompt_appends_submit_only_directive() {
        let out = compose_system_prompt("base prompt", true, None);
        assert!(out.starts_with("base prompt"), "original prompt preserved");
        assert!(
            out.contains("SUBMIT-ONLY TURN"),
            "submit-only directive injected"
        );
        assert!(
            out.contains(SUBMIT_STAGE_DELIVERABLE_TOOL),
            "directive names the submit tool the model must call"
        );
    }

    #[test]
    fn compose_system_prompt_appends_forced_stage_run_directive() {
        let out = compose_system_prompt("base prompt", false, Some("stage_run"));
        assert!(out.starts_with("base prompt"), "original prompt preserved");
        assert!(
            out.contains("TOOL-LOCKED TURN"),
            "forced-tool directive injected"
        );
        assert!(
            out.contains("`stage_run`"),
            "directive names the forced tool"
        );
        assert!(
            out.contains("\"orgs\":[]"),
            "stage_run resume should tell the model to let runtime expand the org subtree"
        );
    }

    #[test]
    fn forces_required_for_xiaomi_primary_in_harness_stage_with_tools() {
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", true, false, true, false, None, true),
            Some(ToolChoice::Required)
        );
    }

    #[test]
    fn no_force_outside_harness_stage() {
        // Chat / non-harness task turns keep provider-default `auto`.
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", false, false, true, false, None, true),
            None
        );
    }

    #[test]
    fn no_force_for_sub_agent() {
        // Sub-agents must be able to end on a text-only summary turn.
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", true, true, true, false, None, true),
            None
        );
    }

    #[test]
    fn no_force_without_tools() {
        // Forcing a tool call with no tools available would be unsatisfiable.
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", true, false, false, false, None, true),
            None
        );
    }

    #[test]
    fn no_force_for_reliable_providers() {
        // Reliable native providers keep their existing behavior unchanged.
        assert_eq!(
            resolve_stage_tool_choice("anthropic", true, false, true, false, None, true),
            None
        );
        assert_eq!(
            resolve_stage_tool_choice("openai", true, false, true, false, None, true),
            None
        );
        assert_eq!(
            resolve_stage_tool_choice("nvidia", true, false, true, false, None, true),
            None
        );
    }

    #[test]
    fn forced_stage_run_locks_any_provider_inside_primary_harness_stage() {
        assert_eq!(
            resolve_stage_tool_choice("openai", true, false, true, false, Some("stage_run"), true,),
            Some(ToolChoice::Specific {
                function_names: vec!["stage_run".to_string()],
            })
        );
    }

    #[test]
    fn thinking_mode_suppresses_native_tool_choice_lock() {
        assert_eq!(
            resolve_stage_tool_choice("openai", true, false, true, false, Some("stage_run"), false,),
            None
        );
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", true, false, true, true, None, false),
            None
        );
    }

    // 设计 2026-06-11 · a targeted gate-repair pass locks the choice onto the
    // submit tool — but only under the exact same provider/stage/primary gate
    // as the Required force.
    #[test]
    fn submit_only_tightens_to_specific_submit_tool_for_xiaomi() {
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", true, false, true, true, None, true),
            Some(ToolChoice::Specific {
                function_names: vec![SUBMIT_STAGE_DELIVERABLE_TOOL.to_string()],
            })
        );
    }

    #[test]
    fn submit_only_does_not_leak_outside_the_provider_gate() {
        // Other providers / sub-agents / no-stage turns never get the lock.
        assert_eq!(
            resolve_stage_tool_choice("anthropic", true, false, true, true, None, true),
            None
        );
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", false, false, true, true, None, true),
            None
        );
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", true, true, true, true, None, true),
            None
        );
        assert_eq!(
            resolve_stage_tool_choice("xiaomi", true, false, false, true, None, true),
            None
        );
    }

    #[test]
    fn detects_request_level_thinking_mode() {
        assert!(request_uses_tool_choice_incompatible_thinking_mode(
            "openai", "gpt-4o", false, true, None
        ));
        assert!(request_uses_tool_choice_incompatible_thinking_mode(
            "openai",
            "gpt-4o",
            false,
            false,
            Some(&json!({"enable_thinking": true}))
        ));
        assert!(request_uses_tool_choice_incompatible_thinking_mode(
            "openai",
            "gpt-4o",
            false,
            false,
            Some(&json!({"chat_template_kwargs": {"enable_thinking": true}}))
        ));
        assert!(request_uses_tool_choice_incompatible_thinking_mode(
            "openai",
            "gpt-4o",
            false,
            false,
            Some(&json!({"reasoning": {"effort": "medium"}}))
        ));
        assert!(!request_uses_tool_choice_incompatible_thinking_mode(
            "openai",
            "gpt-4o",
            false,
            false,
            Some(&json!({"reasoning": {"exclude": true}}))
        ));
    }

    #[test]
    fn deepseek_thinking_model_defaults_suppress_native_tool_choice() {
        assert!(request_uses_tool_choice_incompatible_thinking_mode(
            "deepseek",
            "deepseek-v4-flash",
            true,
            false,
            None
        ));
        assert!(!request_uses_tool_choice_incompatible_thinking_mode(
            "deepseek",
            "deepseek-chat",
            true,
            false,
            None
        ));
    }

    #[test]
    fn strips_tool_choice_after_thinking_mode_rejection() {
        assert!(tool_choice_rejected_by_thinking_mode(
            "ProviderError: Thinking mode does not support this tool_choice"
        ));
        assert!(!tool_choice_rejected_by_thinking_mode(
            "ProviderError: request timed out"
        ));

        let mut additional_params = Some(json!({
            "tool_choice": {
                "type": "function",
                "function": {"name": "stage_run"}
            },
            "reasoning": {"effort": "low"}
        }));
        assert!(additional_params_has_tool_choice(&additional_params));
        strip_tool_choice_from_additional_params(&mut additional_params);
        assert!(!additional_params_has_tool_choice(&additional_params));
        assert_eq!(
            additional_params,
            Some(json!({"reasoning": {"effort": "low"}}))
        );
    }
}
