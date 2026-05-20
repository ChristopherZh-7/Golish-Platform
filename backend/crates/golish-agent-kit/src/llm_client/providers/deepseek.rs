//! DeepSeek (OpenAI-compatible) provider builder.

use std::sync::Arc;

use anyhow::Result;
use rig::client::CompletionClient;
use rig::providers::openai as rig_openai;
use tokio::sync::RwLock;

use crate::llm_client::{
    create_shared_components, AgentBridgeComponents, DeepSeekClientConfig, LlmClient,
    SharedComponentsConfig,
};

/// Create components for a DeepSeek direct API client.
pub async fn create_deepseek_components(
    config: DeepSeekClientConfig<'_>,
    shared_config: SharedComponentsConfig,
) -> Result<AgentBridgeComponents> {
    let base_url = config
        .base_url
        .unwrap_or(golish_llm_providers::DEEPSEEK_DEFAULT_BASE_URL);

    tracing::info!(
        target: "golish::provider",
        "[DeepSeek] Creating client for model={} base_url={}",
        config.model, base_url
    );

    let deepseek_client = rig_openai::Client::builder()
        .api_key(config.api_key)
        .base_url(base_url)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create DeepSeek client: {}", e))?;
    let completion_model = deepseek_client
        .completions_api()
        .completion_model(config.model);
    let client = LlmClient::RigDeepSeek(completion_model);

    let shared = create_shared_components(&config.workspace, config.model, shared_config).await;

    Ok(AgentBridgeComponents {
        workspace: Arc::new(RwLock::new(config.workspace)),
        provider_name: "deepseek".to_string(),
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
