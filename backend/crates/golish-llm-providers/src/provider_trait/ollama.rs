//! `OllamaProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;
use rig::client::CompletionClient;

use super::super::LlmClient;
use super::LlmProvider;


/// Ollama provider implementation (local inference).
pub struct OllamaProviderImpl {
    pub base_url: Option<String>,
}

#[async_trait]
impl LlmProvider for OllamaProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::Ollama
    }

    fn provider_name(&self) -> &'static str {
        "ollama"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        use rig::providers::ollama as rig_ollama;

        // TODO: Support custom base_url when rig-ollama adds support
        if self.base_url.is_some() {
            tracing::warn!("Custom base_url is not yet supported for Ollama provider, ignoring");
        }

        let client = rig_ollama::Client::builder()
            .api_key(rig::client::Nothing)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create Ollama client: {}", e))?;
        let completion_model = client.completion_model(model);

        Ok(LlmClient::RigOllama(completion_model))
    }

    fn validate_credentials(&self) -> Result<()> {
        // Ollama doesn't require credentials
        Ok(())
    }
}
