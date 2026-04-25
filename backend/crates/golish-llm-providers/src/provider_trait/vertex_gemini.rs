//! `VertexGeminiProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;

use super::super::LlmClient;
use super::LlmProvider;


/// Vertex AI Gemini provider implementation.
pub struct VertexGeminiProviderImpl {
    pub credentials_path: Option<String>,
    pub project_id: String,
    pub location: String,
    pub include_thoughts: bool,
}

#[async_trait]
impl LlmProvider for VertexGeminiProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::VertexGemini
    }

    fn provider_name(&self) -> &'static str {
        "vertex_gemini"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        let vertex_client = match &self.credentials_path {
            Some(path) => rig_gemini_vertex::Client::from_service_account(
                path,
                &self.project_id,
                &self.location,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create Vertex Gemini client: {}", e))?,
            None => rig_gemini_vertex::Client::from_env(&self.project_id, &self.location)
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Failed to create Vertex Gemini client from env: {}", e)
                })?,
        };

        let completion_model = vertex_client
            .completion_model(model)
            .with_include_thoughts(self.include_thoughts);
        Ok(LlmClient::VertexGemini(completion_model))
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.project_id.is_empty() {
            anyhow::bail!("Vertex Gemini project_id not configured");
        }
        if self.location.is_empty() {
            anyhow::bail!("Vertex Gemini location not configured");
        }
        Ok(())
    }
}
