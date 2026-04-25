//! Gemini-on-Vertex (Google Vertex AI) provider builder.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::llm_client::{
    create_shared_components, rig_gemini_vertex, AgentBridgeComponents, LlmClient,
    SharedComponentsConfig, VertexGeminiClientConfig,
};

/// Create components for a Vertex AI Gemini based client.
///
/// The `shared_config` parameter allows configuring context management and shell override.
/// If not provided, defaults are used (context management disabled, no shell override).
pub async fn create_vertex_gemini_components(
    config: VertexGeminiClientConfig<'_>,
    shared_config: SharedComponentsConfig,
) -> Result<AgentBridgeComponents> {
    let vertex_client = match config.credentials_path {
        Some(path) => rig_gemini_vertex::Client::from_service_account(
            path,
            config.project_id,
            config.location,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create Vertex AI Gemini client: {}", e))?,
        None => rig_gemini_vertex::Client::from_env(config.project_id, config.location)
            .await
            .map_err(|e| {
                anyhow::anyhow!("Failed to create Vertex AI Gemini client from env: {}", e)
            })?,
    };

    let completion_model = vertex_client
        .completion_model(config.model)
        .with_include_thoughts(config.include_thoughts);

    let shared = create_shared_components(&config.workspace, config.model, shared_config).await;

    Ok(AgentBridgeComponents {
        workspace: Arc::new(RwLock::new(config.workspace)),
        provider_name: "vertex_gemini".to_string(),
        model_name: config.model.to_string(),
        tool_registry: shared.tool_registry,
        client: Arc::new(RwLock::new(LlmClient::VertexGemini(completion_model))),
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
