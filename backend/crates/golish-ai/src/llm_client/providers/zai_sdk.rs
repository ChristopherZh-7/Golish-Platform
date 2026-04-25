//! Z.AI native-SDK provider builder.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::llm_client::{
    create_shared_components, rig_zai_sdk, AgentBridgeComponents, LlmClient,
    SharedComponentsConfig, ZaiSdkClientConfig,
};

/// Create AgentBridge components for Z.AI via native SDK implementation.
///
/// Uses the rig-zai-sdk crate for direct Z.AI API access with streaming support.
pub async fn create_zai_sdk_components(
    config: ZaiSdkClientConfig<'_>,
    shared_config: SharedComponentsConfig,
) -> Result<AgentBridgeComponents> {
    let zai_client = rig_zai_sdk::Client::with_config(
        config.api_key,
        config.base_url.map(|s| s.to_string()),
        config.source_channel.map(|s| s.to_string()),
    );
    let completion_model = zai_client.completion_model(config.model);
    let client = LlmClient::RigZaiSdk(completion_model);

    let shared = create_shared_components(&config.workspace, config.model, shared_config).await;

    Ok(AgentBridgeComponents {
        workspace: Arc::new(RwLock::new(config.workspace)),
        provider_name: "zai_sdk".to_string(),
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
