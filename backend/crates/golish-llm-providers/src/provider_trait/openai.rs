//! `OpenAiProviderImpl`: provider-specific implementation of [`super::LlmProvider`].

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;
use rig::client::CompletionClient;

use super::super::LlmClient;
use super::LlmProvider;


// =============================================================================
// Provider Implementations
// =============================================================================

/// OpenAI provider implementation.
pub struct OpenAiProviderImpl {
    pub api_key: String,
    pub base_url: Option<String>,
    pub reasoning_effort: Option<String>,
    pub enable_web_search: bool,
    pub web_search_context_size: String,
}

#[async_trait]
impl LlmProvider for OpenAiProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::Openai
    }

    fn provider_name(&self) -> &'static str {
        "openai"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        use crate::rig_openai_responses;
        use rig::providers::openai as rig_openai;

        let capabilities = self.get_capabilities(model);

        tracing::info!(
            target: "golish::provider",
            "[OpenAiProvider] Creating client for model={} is_reasoning={}",
            model,
            capabilities.is_reasoning_model
        );

        if capabilities.is_reasoning_model {
            let client = rig_openai_responses::Client::new(&self.api_key);
            let mut completion_model = client.completion_model(model);

            // Set reasoning effort if provided
            if let Some(ref effort_str) = self.reasoning_effort {
                let effort = match effort_str.to_lowercase().as_str() {
                    "low" => rig_openai_responses::ReasoningEffort::Low,
                    "high" => rig_openai_responses::ReasoningEffort::High,
                    "extra_high" | "xhigh" => rig_openai_responses::ReasoningEffort::ExtraHigh,
                    _ => rig_openai_responses::ReasoningEffort::Medium,
                };
                completion_model = completion_model.with_reasoning_effort(effort);
            }

            Ok(LlmClient::OpenAiReasoning(completion_model))
        } else {
            let client = rig_openai::Client::new(&self.api_key)
                .map_err(|e| anyhow::anyhow!("Failed to create OpenAI client: {}", e))?;
            let completion_model = client.completion_model(model);
            Ok(LlmClient::RigOpenAiResponses(completion_model))
        }
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.api_key.is_empty() {
            anyhow::bail!("OpenAI API key not configured");
        }
        Ok(())
    }
}
