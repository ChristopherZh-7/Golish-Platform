//! [`SettingsLlmOneShot`] — the composition-side implementation of the
//! [`LlmOneShot`](golish_app_core::ports::llm::LlmOneShot) port.
//!
//! It builds a DeepSeek client on demand from `~/.golish/settings.toml`
//! (`[ai.deepseek]`) via [`golish_llm_providers::create_client_for_model`] and
//! issues a single non-streaming completion with
//! [`golish_llm_providers::LlmClient::one_shot_completion`]. This is the vehicle
//! that lets the JS collect/extract bridge tools call AI internally while
//! `golish-pentest-app` only depends on the abstract port.

use std::sync::Arc;

use anyhow::Result;
use golish_app_core::ports::llm::LlmOneShot;
use golish_llm_providers::create_client_for_model;
use golish_settings::schema::AiProvider;
use golish_settings::SettingsManager;

/// Default DeepSeek model for tool-internal one-shot calls (see
/// `resources/llm-models/deepseek.json`).
const DEFAULT_MODEL: &str = "deepseek-v4-flash";

/// One-shot LLM handle fixed to DeepSeek, configured from the app settings.
pub struct SettingsLlmOneShot {
    settings: Arc<SettingsManager>,
    model: String,
}

impl SettingsLlmOneShot {
    pub fn new(settings: Arc<SettingsManager>) -> Self {
        Self {
            settings,
            model: DEFAULT_MODEL.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl LlmOneShot for SettingsLlmOneShot {
    async fn complete(
        &self,
        system_prompt: &str,
        user_message: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
    ) -> Result<String> {
        let settings = self.settings.get().await;
        let client = create_client_for_model(AiProvider::Deepseek, &self.model, &settings).await?;
        client
            .one_shot_completion(system_prompt, user_message, temperature, max_tokens)
            .await
    }

    async fn is_available(&self) -> bool {
        let settings = self.settings.get().await;
        settings
            .ai
            .deepseek
            .api_key
            .as_deref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false)
    }
}
