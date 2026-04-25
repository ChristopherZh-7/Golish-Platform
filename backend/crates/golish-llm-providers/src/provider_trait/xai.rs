//! `XaiProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;
use rig::client::CompletionClient;

use super::super::LlmClient;
use super::LlmProvider;


/// xAI (Grok) provider implementation.
pub struct XaiProviderImpl {
    pub api_key: String,
}

#[async_trait]
impl LlmProvider for XaiProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::Xai
    }

    fn provider_name(&self) -> &'static str {
        "xai"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        use rig::providers::xai as rig_xai;

        let client = rig_xai::Client::builder()
            .api_key(&self.api_key)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create xAI client: {}", e))?;
        let completion_model = client.completion_model(model);

        Ok(LlmClient::RigXai(completion_model))
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.api_key.is_empty() {
            anyhow::bail!("xAI API key not configured");
        }
        Ok(())
    }
}
