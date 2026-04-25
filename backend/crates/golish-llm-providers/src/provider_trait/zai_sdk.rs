//! `ZaiSdkProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;

use super::super::LlmClient;
use super::LlmProvider;


/// Z.AI SDK provider implementation.
pub struct ZaiSdkProviderImpl {
    pub api_key: String,
    pub base_url: Option<String>,
    pub source_channel: Option<String>,
}

#[async_trait]
impl LlmProvider for ZaiSdkProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::ZaiSdk
    }

    fn provider_name(&self) -> &'static str {
        "zai_sdk"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        use crate::rig_zai_sdk;

        let client = rig_zai_sdk::Client::with_config(
            &self.api_key,
            self.base_url.clone(),
            self.source_channel.clone(),
        );
        let completion_model = client.completion_model(model);

        Ok(LlmClient::RigZaiSdk(completion_model))
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.api_key.is_empty() {
            anyhow::bail!("Z.AI API key not configured");
        }
        Ok(())
    }
}
