//! `GroqProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;
use rig::client::CompletionClient;

use super::super::LlmClient;
use super::LlmProvider;


/// Groq provider implementation.
pub struct GroqProviderImpl {
    pub api_key: String,
}

#[async_trait]
impl LlmProvider for GroqProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::Groq
    }

    fn provider_name(&self) -> &'static str {
        "groq"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        use rig::providers::groq as rig_groq;

        let client = rig_groq::Client::builder()
            .api_key(&self.api_key)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create Groq client: {}", e))?;
        let completion_model = client.completion_model(model);

        Ok(LlmClient::RigGroq(completion_model))
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.api_key.is_empty() {
            anyhow::bail!("Groq API key not configured");
        }
        Ok(())
    }
}
