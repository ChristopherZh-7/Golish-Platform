//! xAI (Grok) provider builder.

use std::sync::Arc;

use anyhow::Result;
use rig::client::CompletionClient;
use rig::providers::xai as rig_xai;
use tokio::sync::RwLock;

use crate::llm_client::{
    create_shared_components, AgentBridgeComponents, LlmClient, SharedComponentsConfig,
    XaiClientConfig,
};

/// Create components for an xAI (Grok) based client.
///
/// The `shared_config` parameter allows configuring context management and shell override.
/// If not provided, defaults are used (context management disabled, no shell override).
pub async fn create_xai_components(
    config: XaiClientConfig<'_>,
    shared_config: SharedComponentsConfig,
) -> Result<AgentBridgeComponents> {
    let xai_client = rig_xai::Client::new(config.api_key)
        .map_err(|e| anyhow::anyhow!("Failed to create xAI client: {}", e))?;
    let completion_model = xai_client.completion_model(config.model);
    let client = LlmClient::RigXai(completion_model);

    let shared = create_shared_components(&config.workspace, config.model, shared_config).await;

    Ok(AgentBridgeComponents {
        workspace: Arc::new(RwLock::new(config.workspace)),
        provider_name: "xai".to_string(),
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
