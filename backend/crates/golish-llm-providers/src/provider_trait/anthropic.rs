//! `AnthropicProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;
use rig::client::CompletionClient;

use super::super::LlmClient;
use super::LlmProvider;


/// Anthropic provider implementation (direct API).
pub struct AnthropicProviderImpl {
    pub api_key: String,
}

#[async_trait]
impl LlmProvider for AnthropicProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::Anthropic
    }

    fn provider_name(&self) -> &'static str {
        "anthropic"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        use rig::providers::anthropic as rig_anthropic;

        let client = rig_anthropic::Client::new(&self.api_key)
            .map_err(|e| anyhow::anyhow!("Failed to create Anthropic client: {}", e))?;
        let completion_model = client.completion_model(model);

        Ok(LlmClient::RigAnthropic(completion_model))
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.api_key.is_empty() {
            anyhow::bail!("Anthropic API key not configured");
        }
        Ok(())
    }
}
