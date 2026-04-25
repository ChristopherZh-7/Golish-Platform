//! Anthropic-on-Vertex (Google Vertex AI) provider builder.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::llm_client::{
    create_shared_components, AgentBridgeComponents, LlmClient, SharedComponentsConfig,
    VertexAnthropicClientConfig,
};

/// Create components for a Vertex AI Anthropic based client.
///
/// The `shared_config` parameter allows configuring context management and shell override.
/// If not provided, defaults are used (context management disabled, no shell override).
pub async fn create_vertex_components(
    config: VertexAnthropicClientConfig<'_>,
    shared_config: SharedComponentsConfig,
) -> Result<AgentBridgeComponents> {
    let vertex_client = match config.credentials_path {
        Some(path) => rig_anthropic_vertex::Client::from_service_account(
            path,
            config.project_id,
            config.location,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create Vertex AI client: {}", e))?,
        None => rig_anthropic_vertex::Client::from_env(config.project_id, config.location)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create Vertex AI client from env: {}", e))?,
    };

    // Enable extended thinking with default budget (10,000 tokens)
    // When thinking is enabled, temperature is automatically set to 1
    // Also enable Claude's native web search (web_search_20250305)
    let mut completion_model = vertex_client
        .completion_model(config.model)
        .with_default_thinking()
        .with_web_search();

    // Enable 1M token context window (beta) for supported models
    if config.model.contains("opus-4-6") || config.model.contains("sonnet-4-6") {
        completion_model = completion_model.with_context_1m();
    }

    let shared = create_shared_components(&config.workspace, config.model, shared_config).await;

    Ok(AgentBridgeComponents {
        workspace: Arc::new(RwLock::new(config.workspace)),
        provider_name: "anthropic_vertex".to_string(),
        model_name: config.model.to_string(),
        tool_registry: shared.tool_registry,
        client: Arc::new(RwLock::new(LlmClient::VertexAnthropic(completion_model))),
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
