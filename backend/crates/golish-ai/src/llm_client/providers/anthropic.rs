//! Direct Anthropic API provider builder.

use std::sync::Arc;

use anyhow::Result;
use rig::client::CompletionClient;
use rig::providers::anthropic as rig_anthropic;
use tokio::sync::RwLock;

use crate::llm_client::{
    create_shared_components, AgentBridgeComponents, AnthropicClientConfig, LlmClient,
    SharedComponentsConfig,
};

/// Create components for a direct Anthropic API client.
///
/// The `shared_config` parameter allows configuring context management and shell override.
/// If not provided, defaults are used (context management disabled, no shell override).
pub async fn create_anthropic_components(
    config: AnthropicClientConfig<'_>,
    shared_config: SharedComponentsConfig,
) -> Result<AgentBridgeComponents> {
    let anthropic_client = rig_anthropic::Client::new(config.api_key)
        .map_err(|e| anyhow::anyhow!("Failed to create Anthropic client: {}", e))?;
    let completion_model = anthropic_client.completion_model(config.model);
    let client = LlmClient::RigAnthropic(completion_model);

    let shared = create_shared_components(&config.workspace, config.model, shared_config).await;

    Ok(AgentBridgeComponents {
        workspace: Arc::new(RwLock::new(config.workspace)),
        provider_name: "anthropic".to_string(),
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
