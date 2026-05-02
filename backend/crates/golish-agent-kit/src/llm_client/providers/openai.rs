//! OpenAI provider builder (handles both Responses API + custom reasoning route).

use std::sync::Arc;

use anyhow::Result;
use rig::client::CompletionClient;
use rig::providers::openai as rig_openai;
use tokio::sync::RwLock;

use crate::llm_client::{
    create_shared_components, AgentBridgeComponents, LlmClient, OpenAiClientConfig,
    SharedComponentsConfig,
};

/// Create components for an OpenAI-based client.
///
/// The `shared_config` parameter allows configuring context management and shell override.
/// If not provided, defaults are used (context management disabled, no shell override).
///
/// For reasoning models (o1, o3, o4, gpt-5.x), this uses a custom provider with explicit
/// streaming event separation to ensure reasoning deltas are never mixed with text deltas.
pub async fn create_openai_components(
    config: OpenAiClientConfig<'_>,
    shared_config: SharedComponentsConfig,
) -> Result<AgentBridgeComponents> {
    // Note: rig-core's OpenAI client doesn't support custom base URLs directly.
    // The base_url config option is reserved for future use or alternative clients.
    if config.base_url.is_some() {
        tracing::warn!("Custom base_url is not yet supported for OpenAI provider, ignoring");
    }

    let is_reasoning = rig_openai_responses::is_reasoning_model(config.model);

    tracing::info!(
        target: "golish::provider",
        "╔══════════════════════════════════════════════════════════════╗"
    );
    tracing::info!(
        target: "golish::provider",
        "║ OpenAI Provider Selection                                    ║"
    );
    tracing::info!(
        target: "golish::provider",
        "╠══════════════════════════════════════════════════════════════╣"
    );
    tracing::info!(
        target: "golish::provider",
        "║ Model: {:<54}║",
        config.model
    );
    tracing::info!(
        target: "golish::provider",
        "║ Is Reasoning Model: {:<41}║",
        if is_reasoning { "YES" } else { "NO" }
    );

    let (client, provider_name) = if is_reasoning {
        tracing::info!(
            target: "golish::provider",
            "║ Provider: rig-openai-responses (custom)                      ║"
        );
        tracing::info!(
            target: "golish::provider",
            "║ Features: Explicit reasoning/text event separation           ║"
        );

        let openai_client = rig_openai_responses::Client::new(config.api_key);
        let mut completion_model = openai_client.completion_model(config.model);

        if let Some(effort_str) = config.reasoning_effort {
            let effort = match effort_str.to_lowercase().as_str() {
                "low" => rig_openai_responses::ReasoningEffort::Low,
                "high" => rig_openai_responses::ReasoningEffort::High,
                "extra_high" | "xhigh" => rig_openai_responses::ReasoningEffort::ExtraHigh,
                _ => rig_openai_responses::ReasoningEffort::Medium,
            };
            completion_model = completion_model.with_reasoning_effort(effort);
            tracing::info!(
                target: "golish::provider",
                "║ Reasoning Effort: {:<43}║",
                effort_str.to_uppercase()
            );
        }

        tracing::info!(
            target: "golish::provider",
            "╚══════════════════════════════════════════════════════════════╝"
        );

        (
            LlmClient::OpenAiReasoning(completion_model),
            "openai_reasoning".to_string(),
        )
    } else {
        tracing::info!(
            target: "golish::provider",
            "║ Provider: rig-core responses_api (built-in)                  ║"
        );
        tracing::info!(
            target: "golish::provider",
            "╚══════════════════════════════════════════════════════════════╝"
        );

        // Use rig-core's built-in Responses API for non-reasoning models
        let openai_client = rig_openai::Client::new(config.api_key)
            .map_err(|e| anyhow::anyhow!("Failed to create OpenAI client: {}", e))?;

        // The Responses API has better tool support. Our sanitize_schema function handles
        // strict mode compatibility by making optional parameters nullable.
        let completion_model = openai_client.completion_model(config.model);
        (
            LlmClient::RigOpenAiResponses(completion_model),
            "openai_responses".to_string(),
        )
    };

    let shared = create_shared_components(&config.workspace, config.model, shared_config).await;

    let openai_web_search_config = if config.enable_web_search {
        tracing::info!(
            "OpenAI web search enabled with context_size={}",
            config.web_search_context_size
        );
        Some(golish_llm_providers::OpenAiWebSearchConfig {
            search_context_size: config.web_search_context_size.to_string(),
            user_location: None, // Could add user location from settings later
        })
    } else {
        None
    };

    Ok(AgentBridgeComponents {
        workspace: Arc::new(RwLock::new(config.workspace)),
        // Provider name distinguishes between reasoning and non-reasoning variants
        provider_name,
        model_name: config.model.to_string(),
        tool_registry: shared.tool_registry,
        client: Arc::new(RwLock::new(client)),
        sub_agent_registry: shared.sub_agent_registry,
        approval_recorder: shared.approval_recorder,
        tool_policy_manager: shared.tool_policy_manager,
        context_manager: shared.context_manager,
        loop_detector: shared.loop_detector,
        openai_web_search_config,
        openai_reasoning_effort: config.reasoning_effort.map(|s| s.to_string()),
        model_factory: None,
        openrouter_provider_preferences: None,
    })
}
