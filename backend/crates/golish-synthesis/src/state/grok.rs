//! Grok (xAI) state synthesizer.
//!
//! Talks to `https://api.x.ai/v1/chat/completions` using either `GROK_API_KEY`
//! or `XAI_API_KEY` environment variables when no explicit key is configured.

use anyhow::{bail, Context, Result};
use golish_settings::schema::SynthesisGrokSettings;

use super::{StateSynthesisInput, StateSynthesisResult, StateSynthesizer};
use crate::prompts::STATE_UPDATE_SYSTEM_PROMPT;

/// Grok-based state synthesizer.
pub struct GrokStateSynthesizer {
    api_key: String,
    model: String,
}

impl GrokStateSynthesizer {
    pub fn new(config: &SynthesisGrokSettings) -> Result<Self> {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("GROK_API_KEY").ok())
            .or_else(|| std::env::var("XAI_API_KEY").ok())
            .context("Grok API key not configured")?;

        Ok(Self {
            api_key,
            model: config.model.clone(),
        })
    }
}

#[async_trait::async_trait]
impl StateSynthesizer for GrokStateSynthesizer {
    async fn synthesize_state(&self, input: &StateSynthesisInput) -> Result<StateSynthesisResult> {
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
            .post("https://api.x.ai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send state synthesis request to Grok")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Grok API error ({}): {}", status, body);
        }

        let response_body: serde_json::Value = response.json().await?;
        let state_body = response_body["choices"][0]["message"]["content"]
            .as_str()
            .context("Invalid response format from Grok")?
            .trim()
            .to_string();

        Ok(StateSynthesisResult {
            state_body,
            backend: "grok".to_string(),
        })
    }

    fn backend_name(&self) -> &'static str {
        "grok"
    }
}
