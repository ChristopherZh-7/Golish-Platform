//! [`LlmClientFactory`] — caching factory for sub-agent model overrides.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use golish_llm_providers::LlmClient;
use golish_settings::schema::AiProvider;

/// Factory for creating and caching LLM client instances.
///
/// Used primarily for sub-agent model overrides, where a sub-agent might use
/// a different model than the main agent.
pub struct LlmClientFactory {
    /// Cached clients by (provider_name, model_name) key
    cache: RwLock<HashMap<(String, String), Arc<LlmClient>>>,
    /// Settings manager for credential lookup
    settings_manager: Arc<golish_settings::SettingsManager>,
}

impl LlmClientFactory {
    /// Create a new factory with settings manager.
    pub fn new(settings_manager: Arc<golish_settings::SettingsManager>) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            settings_manager,
        }
    }

    /// Get or create an LLM client for the specified provider and model.
    ///
    /// Clients are cached by (provider, model) to avoid recreating them.
    /// Returns an error if credentials are missing or invalid.
    pub async fn get_or_create(&self, provider: &str, model: &str) -> Result<Arc<LlmClient>> {
        let key = (provider.to_string(), model.to_string());

        {
            let cache = self.cache.read().await;
            if let Some(client) = cache.get(&key) {
                tracing::debug!("LlmClientFactory: cache hit for {}/{}", provider, model);
                return Ok(client.clone());
            }
        }

        tracing::info!(
            "LlmClientFactory: creating client for {}/{}",
            provider,
            model
        );
        let client = self.create_client(provider, model).await?;
        let client = Arc::new(client);

        self.cache.write().await.insert(key, client.clone());
        Ok(client)
    }

    /// Create a new LLM client for the given provider and model.
    ///
    /// Uses the unified provider trait abstraction from `golish-llm-providers`.
    async fn create_client(&self, provider: &str, model: &str) -> Result<LlmClient> {
        let settings = self.settings_manager.get().await;

        let ai_provider: AiProvider = provider
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid provider '{}': {}", provider, e))?;

        golish_llm_providers::create_client_for_model(ai_provider, model, &settings).await
    }
}
