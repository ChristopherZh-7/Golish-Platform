//! `XiaomiProviderImpl`: provider-specific implementation of [`super::LlmProvider`].
//!
//! Xiaomi MiMo Token Plan exposes two wire protocols on the same API key:
//! OpenAI Chat Completions and Anthropic Messages. The wire is chosen per
//! request from (in order): the model id's `@suffix`, the provider-level
//! `default_protocol` setting, then OpenAI-compatible as the safe default.
//!
//! See `docs/design/2026-05-27-add-xiaomi-mimo-provider.md`.

use anyhow::Result;
use async_trait::async_trait;

use golish_models::AiProvider;
use rig::client::CompletionClient;

use super::super::LlmClient;
use super::LlmProvider;
use crate::xiaomi::{resolve_protocol, strip_protocol_suffix, XiaomiProtocol, XiaomiRegion};

/// Xiaomi MiMo Token Plan provider (OpenAI + Anthropic dual-compatible).
pub struct XiaomiProviderImpl {
    pub api_key: String,
    pub region: XiaomiRegion,
    pub default_protocol: XiaomiProtocol,
    /// Optional explicit override for the OpenAI-compatible base URL.
    pub openai_base_url: Option<String>,
    /// Optional explicit override for the Anthropic-compatible base URL.
    pub anthropic_base_url: Option<String>,
}

impl XiaomiProviderImpl {
    fn effective_openai_base_url(&self) -> String {
        self.openai_base_url
            .clone()
            .unwrap_or_else(|| self.region.openai_base_url().to_string())
    }

    fn effective_anthropic_base_url(&self) -> String {
        self.anthropic_base_url
            .clone()
            .unwrap_or_else(|| self.region.anthropic_base_url().to_string())
    }
}

#[async_trait]
impl LlmProvider for XiaomiProviderImpl {
    fn provider_type(&self) -> AiProvider {
        AiProvider::Xiaomi
    }

    fn provider_name(&self) -> &'static str {
        "xiaomi"
    }

    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        let protocol = resolve_protocol(model, self.default_protocol);
        let upstream_model = strip_protocol_suffix(model);

        match protocol {
            XiaomiProtocol::OpenaiCompatible | XiaomiProtocol::Auto => {
                use rig::providers::openai as rig_openai;

                let base_url = self.effective_openai_base_url();
                let client = rig_openai::Client::builder()
                    .api_key(&self.api_key)
                    .base_url(&base_url)
                    .build()
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to create Xiaomi (OpenAI-compatible) client: {e}")
                    })?;
                let completion_model = client.completions_api().completion_model(upstream_model);
                Ok(LlmClient::RigXiaomi(completion_model))
            }
            XiaomiProtocol::AnthropicCompatible => {
                use rig::providers::anthropic as rig_anthropic;

                let base_url = self.effective_anthropic_base_url();
                let client = rig_anthropic::Client::builder()
                    .api_key(&self.api_key)
                    .base_url(&base_url)
                    .build()
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to create Xiaomi (Anthropic-compatible) client: {e}"
                        )
                    })?;
                let completion_model = client.completion_model(upstream_model);
                Ok(LlmClient::RigXiaomiAnthropic(completion_model))
            }
        }
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.api_key.is_empty() {
            anyhow::bail!("Xiaomi MiMo API key not configured");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_impl(api_key: &str) -> XiaomiProviderImpl {
        XiaomiProviderImpl {
            api_key: api_key.to_string(),
            region: XiaomiRegion::Cn,
            default_protocol: XiaomiProtocol::Auto,
            openai_base_url: None,
            anthropic_base_url: None,
        }
    }

    #[test]
    fn provider_metadata_is_stable() {
        let p = make_impl("tp-test");
        assert_eq!(p.provider_type(), AiProvider::Xiaomi);
        assert_eq!(p.provider_name(), "xiaomi");
    }

    #[test]
    fn validate_credentials_rejects_empty_key() {
        let empty = make_impl("");
        assert!(empty.validate_credentials().is_err());

        let valid = make_impl("tp-some-key");
        assert!(valid.validate_credentials().is_ok());
    }

    #[test]
    fn effective_urls_default_to_region() {
        let p = make_impl("tp-test");
        assert_eq!(
            p.effective_openai_base_url(),
            "https://token-plan-cn.xiaomimimo.com/v1"
        );
        assert_eq!(
            p.effective_anthropic_base_url(),
            "https://token-plan-cn.xiaomimimo.com/anthropic"
        );
    }

    #[test]
    fn effective_urls_honor_overrides() {
        let p = XiaomiProviderImpl {
            api_key: "tp-test".to_string(),
            region: XiaomiRegion::Cn,
            default_protocol: XiaomiProtocol::Auto,
            openai_base_url: Some("https://proxy.example.com/v1".to_string()),
            anthropic_base_url: Some("https://proxy.example.com/anthropic".to_string()),
        };
        assert_eq!(
            p.effective_openai_base_url(),
            "https://proxy.example.com/v1"
        );
        assert_eq!(
            p.effective_anthropic_base_url(),
            "https://proxy.example.com/anthropic"
        );
    }
}
