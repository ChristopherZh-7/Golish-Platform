//! Xiaomi MiMo (OpenAI + Anthropic dual-compatible) provider builder.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use golish_llm_providers::xiaomi::{XiaomiProtocol, XiaomiRegion};
use golish_llm_providers::{LlmProvider, XiaomiProviderImpl};

use crate::llm_client::{
    create_shared_components, AgentBridgeComponents, SharedComponentsConfig, XiaomiClientConfig,
};

/// Create components for a Xiaomi MiMo client.
///
/// Routes per model id `@suffix` → settings `default_protocol` → OpenAI-compatible
/// (see [`golish_llm_providers::xiaomi::resolve_protocol`]). The same
/// XiaomiProviderImpl path the live probe uses.
pub async fn create_xiaomi_components(
    config: XiaomiClientConfig<'_>,
    shared_config: SharedComponentsConfig,
) -> Result<AgentBridgeComponents> {
    let region = XiaomiRegion::from_settings(config.region);
    let protocol = XiaomiProtocol::from_settings(config.default_protocol);

    tracing::info!(
        target: "golish::provider",
        "[Xiaomi] Creating client for model={} region={:?} default_protocol={:?}",
        config.model, region, protocol
    );

    let provider = XiaomiProviderImpl {
        api_key: config.api_key.to_string(),
        region,
        default_protocol: protocol,
        openai_base_url: config.openai_base_url.map(str::to_string),
        anthropic_base_url: config.anthropic_base_url.map(str::to_string),
    };
    provider.validate_credentials()?;
    let client = provider.create_client(config.model).await?;

    let shared = create_shared_components(&config.workspace, config.model, shared_config).await;

    Ok(AgentBridgeComponents {
        workspace: Arc::new(RwLock::new(config.workspace)),
        provider_name: "xiaomi".to_string(),
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
