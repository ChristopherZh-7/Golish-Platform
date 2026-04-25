//! `GeminiProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;
use rig::client::CompletionClient;

use super::super::LlmClient;
use super::LlmProvider;


/// Gemini provider implementation.
pub struct GeminiProviderImpl {
    pub api_key: String,
}

#[async_trait]
impl LlmProvider for GeminiProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::Gemini
    }

    fn provider_name(&self) -> &'static str {
        "gemini"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        use rig::providers::gemini as rig_gemini;

        let client = rig_gemini::Client::new(&self.api_key)
            .map_err(|e| anyhow::anyhow!("Failed to create Gemini client: {}", e))?;
        let completion_model = client.completion_model(model);

        Ok(LlmClient::RigGemini(completion_model))
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.api_key.is_empty() {
            anyhow::bail!("Gemini API key not configured");
        }
        Ok(())
    }
}
