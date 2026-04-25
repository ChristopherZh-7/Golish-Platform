//! `VertexAiProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;

use super::super::LlmClient;
use super::LlmProvider;


/// Vertex AI (Anthropic Claude on Google Cloud) provider implementation.
pub struct VertexAiProviderImpl {
    pub credentials_path: Option<String>,
    pub project_id: String,
    pub location: String,
}

#[async_trait]
impl LlmProvider for VertexAiProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::VertexAi
    }

    fn provider_name(&self) -> &'static str {
        "vertex_ai"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        let vertex_client = match &self.credentials_path {
            Some(path) => rig_anthropic_vertex::Client::from_service_account(
                path,
                &self.project_id,
                &self.location,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create Vertex AI client: {}", e))?,
            None => rig_anthropic_vertex::Client::from_env(&self.project_id, &self.location)
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Failed to create Vertex AI client from env: {}", e)
                })?,
        };

        // Enable extended thinking and web search for Claude on Vertex
        let mut completion_model = vertex_client
            .completion_model(model)
            .with_default_thinking()
            .with_web_search();

        // Enable 1M token context window (beta) for supported models
        if model.contains("opus-4-6") || model.contains("sonnet-4-6") {
            completion_model = completion_model.with_context_1m();
        }

        Ok(LlmClient::VertexAnthropic(completion_model))
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.project_id.is_empty() {
            anyhow::bail!("Vertex AI project_id not configured");
        }
        if self.location.is_empty() {
            anyhow::bail!("Vertex AI location not configured");
        }
        Ok(())
    }
}
