//! `DeepSeekProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;
use rig::client::CompletionClient;

use super::super::LlmClient;
use super::LlmProvider;
use crate::DEEPSEEK_DEFAULT_BASE_URL;

/// DeepSeek provider implementation (OpenAI-compatible Chat Completions API).
pub struct DeepSeekProviderImpl {
    pub api_key: String,
    pub base_url: Option<String>,
}

#[async_trait]
impl LlmProvider for DeepSeekProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::Deepseek
    }

    fn provider_name(&self) -> &'static str {
        "deepseek"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        use rig::providers::openai as rig_openai;

        let base_url = self
            .base_url
            .as_deref()
            .unwrap_or(DEEPSEEK_DEFAULT_BASE_URL);

        let client = rig_openai::Client::builder()
            .api_key(&self.api_key)
            .base_url(base_url)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create DeepSeek client: {}", e))?;
        let completion_model = client.completions_api().completion_model(model);

        Ok(LlmClient::RigDeepSeek(completion_model))
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.api_key.is_empty() {
            anyhow::bail!("DeepSeek API key not configured");
        }
        Ok(())
    }
}
