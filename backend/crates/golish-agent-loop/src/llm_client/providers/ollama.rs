//! Ollama (local) provider builder.

use std::sync::Arc;

use anyhow::Result;
use rig::client::CompletionClient;
use rig::providers::ollama as rig_ollama;
use tokio::sync::RwLock;

use crate::llm_client::{
    create_shared_components, AgentBridgeComponents, LlmClient, OllamaClientConfig,
    SharedComponentsConfig,
};

/// Create components for an Ollama-based client.
///
/// The `shared_config` parameter allows configuring context management and shell override.
/// If not provided, defaults are used (context management disabled, no shell override).
pub async fn create_ollama_components(
    config: OllamaClientConfig<'_>,
    shared_config: SharedComponentsConfig,
) -> Result<AgentBridgeComponents> {
    // Note: rig-core's Ollama client only supports the default localhost:11434 endpoint.
    // The base_url config option is reserved for future use when rig-core adds this feature.
    if config.base_url.is_some() {
        tracing::warn!(
            "Custom base_url is not yet supported for Ollama provider (rig-core defaults to http://localhost:11434), ignoring"
        );
    }

    // Ollama doesn't require an API key, so we use client::Nothing
    let ollama_client = rig_ollama::Client::builder()
        .api_key(rig::client::Nothing)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create Ollama client: {}", e))?;
    let completion_model = ollama_client.completion_model(config.model);
    let client = LlmClient::RigOllama(completion_model);

    let shared = create_shared_components(&config.workspace, config.model, shared_config).await;

    Ok(AgentBridgeComponents {
        workspace: Arc::new(RwLock::new(config.workspace)),
        provider_name: "ollama".to_string(),
        model_name: config.model.to_string(),
        tool_registry: shared.tool_registry,
        client: Arc::new(RwLock::new(client)),
        sub_agent_registry: shared.sub_agent_registry,
        approval_recorder: shared.approval_recorder,
        tool_policy_manager: shared.tool_policy_manager,
        context_manager: shared.context_manager,
        loop_detector: shared.loop_detector,
        openai_web_search_config: None,
        openai_reasoning_effort: None,
        model_factory: None,
        openrouter_provider_preferences: None,
    })
}
