//! OpenAI-backed state synthesizer.
//!
//! Calls the `chat/completions` endpoint of any OpenAI-compatible API
//! (`base_url` is configurable so this also covers Azure OpenAI, vLLM
//! deployments, and so on).

use anyhow::{bail, Context, Result};
use golish_settings::schema::SynthesisOpenAiSettings;

use super::{StateSynthesisInput, StateSynthesisResult, StateSynthesizer};
use crate::prompts::STATE_UPDATE_SYSTEM_PROMPT;

/// OpenAI-based state synthesizer.
pub struct OpenAiStateSynthesizer {
    api_key: String,
    model: String,
    base_url: Option<String>,
}

impl OpenAiStateSynthesizer {
    pub fn new(config: &SynthesisOpenAiSettings) -> Result<Self> {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .context("OpenAI API key not configured")?;

        Ok(Self {
            api_key,
            model: config.model.clone(),
            base_url: config.base_url.clone(),
        })
    }
}

#[async_trait::async_trait]
impl StateSynthesizer for OpenAiStateSynthesizer {
    async fn synthesize_state(&self, input: &StateSynthesisInput) -> Result<StateSynthesisResult> {
        let base_url = self
            .base_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1");

        let client = reqwest::Client::new();

        let request_body = serde_json::json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": STATE_UPDATE_SYSTEM_PROMPT
                },
                {
                    "role": "user",
                    "content": input.build_prompt()
                }
            ],
            "max_tokens": 1500,
            "temperature": 0.3
        });

        let response = client
            .post(format!("{}/chat/completions", base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send state synthesis request to OpenAI")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("OpenAI API error ({}): {}", status, body);
        }

        let response_body: serde_json::Value = response.json().await?;
        let state_body = response_body["choices"][0]["message"]["content"]
            .as_str()
            .context("Invalid response format from OpenAI")?
            .trim()
            .to_string();

        Ok(StateSynthesisResult {
            state_body,
            backend: "openai".to_string(),
        })
    }

    fn backend_name(&self) -> &'static str {
        "openai"
    }
}
