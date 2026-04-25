//! `NvidiaProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;
use rig::client::CompletionClient;

use super::super::LlmClient;
use super::LlmProvider;


/// NVIDIA NIM provider implementation (OpenAI-compatible).
pub struct NvidiaProviderImpl {
    pub api_key: String,
    pub base_url: Option<String>,
}

#[async_trait]
impl LlmProvider for NvidiaProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::Nvidia
    }

    fn provider_name(&self) -> &'static str {
        "nvidia"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        use rig::providers::openai as rig_openai;

        let base_url = self
            .base_url
            .as_deref()
            .unwrap_or("https://integrate.api.nvidia.com/v1");

        tracing::info!(
            target: "golish::provider",
            "[NvidiaProvider] Creating client for model={} base_url={}",
            model, base_url
        );

        let client = rig_openai::Client::builder()
            .api_key(&self.api_key)
            .base_url(base_url)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create NVIDIA NIM client: {}", e))?;
        let completions_client = client.completions_api();
        let completion_model = completions_client.completion_model(model);

        Ok(LlmClient::RigNvidia(completion_model))
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.api_key.is_empty() {
            anyhow::bail!("NVIDIA API key not configured");
        }
        Ok(())
    }
}
