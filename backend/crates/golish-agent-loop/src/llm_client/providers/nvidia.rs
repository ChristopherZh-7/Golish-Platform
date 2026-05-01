//! NVIDIA NIM (OpenAI-compatible) provider builder.

use std::sync::Arc;

use anyhow::Result;
use rig::client::CompletionClient;
use rig::providers::openai as rig_openai;
use tokio::sync::RwLock;

use crate::llm_client::{
    create_shared_components, AgentBridgeComponents, LlmClient, NvidiaClientConfig,
    SharedComponentsConfig,
};

/// Create components for an NVIDIA NIM based client (OpenAI-compatible API).
pub async fn create_nvidia_components(
    config: NvidiaClientConfig<'_>,
    shared_config: SharedComponentsConfig,
) -> Result<AgentBridgeComponents> {
    let base_url = config
        .base_url
        .unwrap_or("https://integrate.api.nvidia.com/v1");

    tracing::info!(
        target: "golish::provider",
        "[NVIDIA NIM] Creating client for model={} base_url={}",
        config.model, base_url
    );

    let nvidia_client = rig_openai::Client::builder()
        .api_key(config.api_key)
        .base_url(base_url)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create NVIDIA NIM client: {}", e))?;
    let completions_client = nvidia_client.completions_api();
    let completion_model = completions_client.completion_model(config.model);
    let client = LlmClient::RigNvidia(completion_model);

    let shared = create_shared_components(&config.workspace, config.model, shared_config).await;

    Ok(AgentBridgeComponents {
        workspace: Arc::new(RwLock::new(config.workspace)),
        provider_name: "nvidia".to_string(),
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
