//! [`LlmProvider`] trait + provider-impl dispatch.
//!
//! Each provider implementation lives in its own submodule. The factory
//! functions ([`create_provider`], [`create_client_for_model`]) below
//! select the right impl by [`AiProvider`].

//! Provider trait abstraction for unified LLM client creation.
//!
//! This module provides a trait-based abstraction over different LLM providers,
//! eliminating code duplication between `create_*_components()` functions and
//! `LlmClientFactory::create_client()`.

use anyhow::Result;
use async_trait::async_trait;
use golish_models::{get_model_capabilities, AiProvider, ModelCapabilities};

use crate::LlmClient;

/// Trait for LLM provider implementations.
///
/// Each provider implements this trait to encapsulate its specific
/// client creation logic. The trait uses the model registry for
/// capability detection instead of string matching.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Get the provider type enum value.
    fn provider_type(&self) -> AiProvider;

    /// Get the provider name for logging.
    fn provider_name(&self) -> &'static str;

    /// Create an LLM client for the given model.
    ///
    /// Uses the model registry to look up capabilities and determine
    /// the appropriate client variant (e.g., reasoning vs standard OpenAI).
    async fn create_client(&self, model: &str) -> Result<LlmClient>;

    /// Validate that the provider has valid credentials configured.
    fn validate_credentials(&self) -> Result<()>;

    /// Get model capabilities from the registry.
    ///
    /// Falls back to provider defaults if the model is not in the registry.
    fn get_capabilities(&self, model: &str) -> ModelCapabilities {
        get_model_capabilities(self.provider_type(), model)
    }
}

/// Configuration for creating providers from settings.
#[derive(Clone)]
pub struct ProviderSettings {
    /// API key (for providers that require one).
    pub api_key: Option<String>,
    /// Base URL override (for providers that support it).
    pub base_url: Option<String>,
    /// Additional provider-specific settings.
    pub extra: ProviderExtraSettings,
}

/// Provider-specific extra settings.
#[derive(Clone, Default)]
pub struct ProviderExtraSettings {
    // Vertex AI specific
    pub credentials_path: Option<String>,
    pub project_id: Option<String>,
    pub location: Option<String>,
    pub include_thoughts: bool,

    // OpenAI specific
    pub reasoning_effort: Option<String>,
    pub enable_web_search: bool,
    pub web_search_context_size: String,

    // Z.AI SDK specific
    pub source_channel: Option<String>,

    // OpenRouter specific
    /// Provider preferences JSON for routing and filtering.
    pub provider_preferences: Option<serde_json::Value>,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            extra: ProviderExtraSettings {
                web_search_context_size: "medium".to_string(),
                ..Default::default()
            },
        }
    }
}

mod anthropic;
mod gemini;
mod groq;
mod nvidia;
mod ollama;
mod openai;
mod openrouter;
mod vertex_ai;
mod vertex_gemini;
mod xai;
mod zai_sdk;

pub use anthropic::AnthropicProviderImpl;
pub use gemini::GeminiProviderImpl;
pub use groq::GroqProviderImpl;
pub use nvidia::NvidiaProviderImpl;
pub use ollama::OllamaProviderImpl;
pub use openai::OpenAiProviderImpl;
pub use openrouter::OpenRouterProviderImpl;
pub use vertex_ai::VertexAiProviderImpl;
pub use vertex_gemini::VertexGeminiProviderImpl;
pub use xai::XaiProviderImpl;
pub use zai_sdk::ZaiSdkProviderImpl;


// =============================================================================
// Provider Factory
// =============================================================================

/// Create a provider implementation from settings.
pub fn create_provider(
    provider_type: AiProvider,
    settings: &ProviderSettings,
) -> Result<Box<dyn LlmProvider>> {
    match provider_type {
        AiProvider::Openai => {
            let api_key = settings
                .api_key
                .clone()
                .ok_or_else(|| anyhow::anyhow!("OpenAI API key required"))?;
            Ok(Box::new(OpenAiProviderImpl {
                api_key,
                base_url: settings.base_url.clone(),
                reasoning_effort: settings.extra.reasoning_effort.clone(),
                enable_web_search: settings.extra.enable_web_search,
                web_search_context_size: settings.extra.web_search_context_size.clone(),
            }))
        }
        AiProvider::Anthropic => {
            let api_key = settings
                .api_key
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Anthropic API key required"))?;
            Ok(Box::new(AnthropicProviderImpl { api_key }))
        }
        AiProvider::VertexAi => {
            let project_id = settings
                .extra
                .project_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Vertex AI project_id required"))?;
            let location = settings
                .extra
                .location
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Vertex AI location required"))?;
            Ok(Box::new(VertexAiProviderImpl {
                credentials_path: settings.extra.credentials_path.clone(),
                project_id,
                location,
            }))
        }
        AiProvider::Openrouter => {
            let api_key = settings
                .api_key
                .clone()
                .ok_or_else(|| anyhow::anyhow!("OpenRouter API key required"))?;
            Ok(Box::new(OpenRouterProviderImpl {
                api_key,
                provider_preferences: settings.extra.provider_preferences.clone(),
            }))
        }
        AiProvider::Ollama => Ok(Box::new(OllamaProviderImpl {
            base_url: settings.base_url.clone(),
        })),
        AiProvider::Gemini => {
            let api_key = settings
                .api_key
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Gemini API key required"))?;
            Ok(Box::new(GeminiProviderImpl { api_key }))
        }
        AiProvider::Groq => {
            let api_key = settings
                .api_key
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Groq API key required"))?;
            Ok(Box::new(GroqProviderImpl { api_key }))
        }
        AiProvider::Xai => {
            let api_key = settings
                .api_key
                .clone()
                .ok_or_else(|| anyhow::anyhow!("xAI API key required"))?;
            Ok(Box::new(XaiProviderImpl { api_key }))
        }
        AiProvider::ZaiSdk => {
            let api_key = settings
                .api_key
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Z.AI API key required"))?;
            Ok(Box::new(ZaiSdkProviderImpl {
                api_key,
                base_url: settings.base_url.clone(),
                source_channel: settings.extra.source_channel.clone(),
            }))
        }
        AiProvider::VertexGemini => {
            let project_id = settings
                .extra
                .project_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Vertex Gemini project_id required"))?;
            let location = settings
                .extra
                .location
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Vertex Gemini location required"))?;
            Ok(Box::new(VertexGeminiProviderImpl {
                credentials_path: settings.extra.credentials_path.clone(),
                project_id,
                location,
                include_thoughts: settings.extra.include_thoughts,
            }))
        }
        AiProvider::Nvidia => {
            let api_key = settings
                .api_key
                .clone()
                .ok_or_else(|| anyhow::anyhow!("NVIDIA API key required"))?;
            Ok(Box::new(NvidiaProviderImpl {
                api_key,
                base_url: settings.base_url.clone(),
            }))
        }
    }
}

/// Extract ProviderSettings from GolishSettings for a given provider.
///
/// This helper function maps the typed settings from `GolishSettings` to the
/// unified `ProviderSettings` structure used by the provider trait.
pub fn extract_provider_settings(
    provider_type: AiProvider,
    settings: &golish_settings::GolishSettings,
) -> ProviderSettings {
    match provider_type {
        AiProvider::Openai => ProviderSettings {
            api_key: settings.ai.openai.api_key.clone(),
            base_url: settings.ai.openai.base_url.clone(),
            extra: ProviderExtraSettings {
                enable_web_search: settings.ai.openai.enable_web_search,
                web_search_context_size: settings.ai.openai.web_search_context_size.clone(),
                ..Default::default()
            },
        },
        AiProvider::Anthropic => ProviderSettings {
            api_key: settings.ai.anthropic.api_key.clone(),
            ..Default::default()
        },
        AiProvider::VertexAi => ProviderSettings {
            extra: ProviderExtraSettings {
                credentials_path: settings.ai.vertex_ai.credentials_path.clone(),
                project_id: settings.ai.vertex_ai.project_id.clone(),
                location: settings.ai.vertex_ai.location.clone(),
                ..Default::default()
            },
            ..Default::default()
        },
        AiProvider::Openrouter => ProviderSettings {
            api_key: settings.ai.openrouter.api_key.clone(),
            extra: ProviderExtraSettings {
                provider_preferences: settings
                    .ai
                    .openrouter
                    .provider_preferences
                    .as_ref()
                    .filter(|p| !p.is_empty())
                    .map(crate::openrouter_preferences_to_json),
                ..Default::default()
            },
            ..Default::default()
        },
        AiProvider::Ollama => ProviderSettings {
            // Ollama base_url is a String, wrap in Option
            base_url: Some(settings.ai.ollama.base_url.clone()),
            ..Default::default()
        },
        AiProvider::Gemini => ProviderSettings {
            api_key: settings.ai.gemini.api_key.clone(),
            ..Default::default()
        },
        AiProvider::Groq => ProviderSettings {
            api_key: settings.ai.groq.api_key.clone(),
            ..Default::default()
        },
        AiProvider::Xai => ProviderSettings {
            api_key: settings.ai.xai.api_key.clone(),
            ..Default::default()
        },
        AiProvider::ZaiSdk => ProviderSettings {
            api_key: settings.ai.zai_sdk.api_key.clone(),
            base_url: settings.ai.zai_sdk.base_url.clone(),
            ..Default::default()
        },
        AiProvider::VertexGemini => ProviderSettings {
            extra: ProviderExtraSettings {
                credentials_path: settings.ai.vertex_gemini.credentials_path.clone(),
                project_id: settings.ai.vertex_gemini.project_id.clone(),
                location: settings.ai.vertex_gemini.location.clone(),
                ..Default::default()
            },
            ..Default::default()
        },
        AiProvider::Nvidia => ProviderSettings {
            api_key: settings.ai.nvidia.api_key.clone(),
            base_url: settings.ai.nvidia.base_url.clone(),
            ..Default::default()
        },
    }
}

/// Create a provider and immediately create a client for the given model.
///
/// This is a convenience function that combines `create_provider()` and
/// `LlmProvider::create_client()` into a single call.
pub async fn create_client_for_model(
    provider_type: AiProvider,
    model: &str,
    settings: &golish_settings::GolishSettings,
) -> Result<crate::LlmClient> {
    let provider_settings = extract_provider_settings(provider_type, settings);
    let provider = create_provider(provider_type, &provider_settings)?;
    provider.create_client(model).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_types() {
        let openai = OpenAiProviderImpl {
            api_key: "test".to_string(),
            base_url: None,
            reasoning_effort: None,
            enable_web_search: false,
            web_search_context_size: "medium".to_string(),
        };
        assert_eq!(openai.provider_type(), AiProvider::Openai);
        assert_eq!(openai.provider_name(), "openai");

        let anthropic = AnthropicProviderImpl {
            api_key: "test".to_string(),
        };
        assert_eq!(anthropic.provider_type(), AiProvider::Anthropic);
        assert_eq!(anthropic.provider_name(), "anthropic");
    }

    #[test]
    fn test_validate_credentials() {
        let empty_openai = OpenAiProviderImpl {
            api_key: "".to_string(),
            base_url: None,
            reasoning_effort: None,
            enable_web_search: false,
            web_search_context_size: "medium".to_string(),
        };
        assert!(empty_openai.validate_credentials().is_err());

        let valid_openai = OpenAiProviderImpl {
            api_key: "sk-test".to_string(),
            base_url: None,
            reasoning_effort: None,
            enable_web_search: false,
            web_search_context_size: "medium".to_string(),
        };
        assert!(valid_openai.validate_credentials().is_ok());

        // Ollama doesn't require credentials
        let ollama = OllamaProviderImpl { base_url: None };
        assert!(ollama.validate_credentials().is_ok());
    }

    #[test]
    fn test_create_provider() {
        let settings = ProviderSettings {
            api_key: Some("test-key".to_string()),
            ..Default::default()
        };

        let provider = create_provider(AiProvider::Openai, &settings).unwrap();
        assert_eq!(provider.provider_type(), AiProvider::Openai);

        // Missing API key should fail
        let empty_settings = ProviderSettings::default();
        assert!(create_provider(AiProvider::Openai, &empty_settings).is_err());

        // Ollama doesn't require API key
        assert!(create_provider(AiProvider::Ollama, &empty_settings).is_ok());
    }
}
