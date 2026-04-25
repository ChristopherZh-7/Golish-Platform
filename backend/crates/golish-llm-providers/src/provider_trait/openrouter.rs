//! `OpenRouterProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;
use rig::client::CompletionClient;

use super::super::LlmClient;
use super::LlmProvider;


/// OpenRouter provider implementation.
pub struct OpenRouterProviderImpl {
    pub api_key: String,
    /// Provider preferences JSON for routing and filtering (optional).
    pub provider_preferences: Option<serde_json::Value>,
}

#[async_trait]
impl LlmProvider for OpenRouterProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::Openrouter
    }

    fn provider_name(&self) -> &'static str {
        "openrouter"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        use rig::providers::openrouter as rig_openrouter;

        let client = rig_openrouter::Client::new(&self.api_key)
            .map_err(|e| anyhow::anyhow!("Failed to create OpenRouter client: {}", e))?;
        let completion_model = client.completion_model(model);

        Ok(LlmClient::RigOpenRouter(completion_model))
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.api_key.is_empty() {
            anyhow::bail!("OpenRouter API key not configured");
        }
        Ok(())
    }
}
